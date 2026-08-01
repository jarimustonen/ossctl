//! Coordinator unit tests: barrier ordering (no target advances early),
//! tag-once, failure-stops-and-journals (no rollback), and resume-ready journal
//! state (idempotent re-entry skips already-landed work). Drift-refusal is a
//! `release cut` (CLI) concern layered on the plan module's re-hash and is
//! covered there; these tests drive the coordinator against fakes.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::*;
use crate::contract::schema::{Adapter, Ecosystem, Registry};
use crate::ports::{
    Clock, CommandOutput, CommandRunner, IdGen, JournalLock, JournalStore, RegistryQuery,
};
use crate::protocol::journal::{Phase, PhaseOutcome, RunStatus};
use crate::protocol::plan::{PlanPhase, PlanTarget, ReleasePlan};
use crate::release::adapters::{EffectCtx, EMPTY_ARTIFACTS};
use crate::release::journal::{Journal, JournalPaths};

// ── In-memory journal store (mirrors the journal module's own fake) ──────────

#[derive(Default)]
struct StoreInner {
    files: HashMap<PathBuf, Vec<u8>>,
    locked: HashSet<PathBuf>,
}

#[derive(Clone, Default)]
struct FakeStore {
    inner: Rc<RefCell<StoreInner>>,
}

struct FakeLock {
    inner: Rc<RefCell<StoreInner>>,
    path: PathBuf,
}
impl JournalLock for FakeLock {}
impl Drop for FakeLock {
    fn drop(&mut self) {
        self.inner.borrow_mut().locked.remove(&self.path);
    }
}

impl JournalStore for FakeStore {
    fn lock_exclusive(&self, lock_path: &Path) -> io::Result<Box<dyn JournalLock>> {
        let mut inner = self.inner.borrow_mut();
        if inner.locked.contains(lock_path) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "locked"));
        }
        inner.locked.insert(lock_path.to_path_buf());
        Ok(Box::new(FakeLock {
            inner: Rc::clone(&self.inner),
            path: lock_path.to_path_buf(),
        }))
    }
    fn append_line(&self, path: &Path, line: &str) -> io::Result<()> {
        let mut inner = self.inner.borrow_mut();
        let buf = inner.files.entry(path.to_path_buf()).or_default();
        buf.extend_from_slice(line.as_bytes());
        buf.push(b'\n');
        Ok(())
    }
    fn read_lines(&self, path: &Path) -> io::Result<Vec<String>> {
        Ok(self
            .inner
            .borrow()
            .files
            .get(path)
            .map(|b| {
                String::from_utf8_lossy(b)
                    .lines()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default())
    }
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        Ok(self.inner.borrow().files.get(path).cloned())
    }
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.inner
            .borrow_mut()
            .files
            .insert(path.to_path_buf(), bytes.to_vec());
        Ok(())
    }
    fn list_dir(&self, _dir: &Path) -> io::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

// ── Effect fakes ─────────────────────────────────────────────────────────────

/// Recording command runner. Succeeds by default; fails any command whose
/// rendered `program args` line contains `fail_contains` (so a build can succeed
/// while the same program's *publish* fails).
struct FakeCmd {
    calls: RefCell<Vec<String>>,
    fail_contains: Option<String>,
    /// Canned `git remote get-url origin` stdout — lets a cut resolve a source
    /// tarball URL. `None` means "no origin" (empty stdout, as a bare repo).
    origin: Option<String>,
}
impl FakeCmd {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail_contains: None,
            origin: None,
        }
    }
    fn failing_on(substr: &str) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail_contains: Some(substr.to_string()),
            origin: None,
        }
    }
    fn with_origin(origin: &str) -> Self {
        Self {
            origin: Some(origin.to_string()),
            ..Self::new()
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}
impl CommandRunner for FakeCmd {
    fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        // Serve the origin remote so `source_tarball` can resolve a GitHub slug.
        if let Some(origin) = &self.origin {
            if program == "git" && args == ["remote", "get-url", "origin"] {
                return Ok(CommandOutput {
                    status: Some(0),
                    stdout: origin.clone(),
                    stderr: String::new(),
                });
            }
        }
        let fails = self
            .fail_contains
            .as_ref()
            .is_some_and(|s| line.contains(s.as_str()));
        Ok(CommandOutput {
            status: Some(i32::from(fails)),
            stdout: String::new(),
            stderr: if fails { "boom".into() } else { String::new() },
        })
    }
}

struct FakeClock(Cell<u64>);
impl Clock for FakeClock {
    fn now_unix(&self) -> u64 {
        let t = self.0.get();
        self.0.set(t + 1);
        t
    }
}

struct FakeIdGen(String);
impl IdGen for FakeIdGen {
    fn new_id(&self) -> String {
        self.0.clone()
    }
}

struct FakeRegistry;
impl RegistryQuery for FakeRegistry {
    fn published_versions(&self, _ecosystem: &str, _package: &str) -> io::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// Recording tagger. Records every call; optionally fails one named step.
struct FakeTagger {
    calls: RefCell<Vec<String>>,
    fail_step: Option<&'static str>,
}
impl FakeTagger {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail_step: None,
        }
    }
    fn failing(step: &'static str) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail_step: Some(step),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}
impl Tagger for FakeTagger {
    fn create_tag(&self, tag: &str, commit: &str, _message: &str) -> io::Result<()> {
        self.calls
            .borrow_mut()
            .push(format!("create:{tag}@{commit}"));
        if self.fail_step == Some("create") {
            return Err(io::Error::other("cannot create tag"));
        }
        Ok(())
    }
    fn push_tag(&self, tag: &str) -> io::Result<()> {
        self.calls.borrow_mut().push(format!("push:{tag}"));
        if self.fail_step == Some("push") {
            return Err(io::Error::other("cannot push tag"));
        }
        Ok(())
    }
    fn create_github_release(&self, tag: &str, _title: &str) -> io::Result<Option<String>> {
        self.calls.borrow_mut().push(format!("release:{tag}"));
        if self.fail_step == Some("release") {
            return Err(io::Error::other("cannot create release"));
        }
        Ok(Some(format!("https://github.com/x/y/releases/{tag}")))
    }
}

/// Records the ordered kinds the coordinator streams, for barrier assertions.
#[derive(Default)]
struct RecordingSink {
    kinds: Vec<EventKind>,
}
impl ProgressSink for RecordingSink {
    fn event(&mut self, event: &JournalEvent) {
        self.kinds.push(event.kind.clone());
    }
}

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn paths() -> JournalPaths {
    JournalPaths::new("/repo/.git/ossctl/releases")
}

fn plan_target(ecosystem: Ecosystem, registry: Registry, adapter: Adapter) -> PlanTarget {
    PlanTarget {
        ecosystem,
        package: Some("tool".to_string()),
        registry,
        adapter,
    }
}

fn two_target_plan() -> ReleasePlan {
    ReleasePlan {
        plan_id: "plan-test".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.2.3".into(),
        targets: vec![
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublish),
            plan_target(Ecosystem::Node, Registry::Npm, Adapter::NpmPublish),
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
    }
}

fn new_journal<'a>(
    store: &'a FakeStore,
    clock: &'a FakeClock,
    idgen: &'a FakeIdGen,
) -> Journal<'a> {
    Journal::create(
        store,
        clock,
        idgen,
        paths(),
        "plan-test".into(),
        "1.2.3".into(),
        vec!["rust".into(), "node".into()],
    )
    .unwrap()
}

/// Index of the first event of a given kind-name in the recorded stream.
fn first_idx(kinds: &[EventKind], pred: impl Fn(&EventKind) -> bool) -> Option<usize> {
    kinds.iter().position(pred)
}

// ── Barrier ordering: no target advances early ───────────────────────────────

#[test]
fn phases_are_strict_barriers() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new();
    let reg = FakeRegistry;
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = RecordingSink::default();
    let mut journal = new_journal(&store, &clock, &idgen);

    execute(&mut journal, &two_target_plan(), &ctx, &tagger, &mut sink).unwrap();

    let k = &sink.kinds;
    // dry-run barrier closes before ANY build target.
    let dry_done = first_idx(k, |e| {
        matches!(
            e,
            EventKind::PhaseCompleted {
                phase: Phase::DryRun,
                ..
            }
        )
    })
    .unwrap();
    let first_built = first_idx(k, |e| matches!(e, EventKind::TargetBuilt { .. })).unwrap();
    assert!(
        dry_done < first_built,
        "a target built before dry-run-all closed"
    );

    // build barrier closes before ANY publish target.
    let build_done = first_idx(k, |e| {
        matches!(
            e,
            EventKind::PhaseCompleted {
                phase: Phase::Build,
                ..
            }
        )
    })
    .unwrap();
    let first_pub = first_idx(k, |e| matches!(e, EventKind::TargetPublished { .. })).unwrap();
    assert!(
        build_done < first_pub,
        "a target published before build-all closed"
    );

    // publish barrier closes before ANY tag step.
    let pub_done = first_idx(k, |e| {
        matches!(
            e,
            EventKind::PhaseCompleted {
                phase: Phase::Publish,
                ..
            }
        )
    })
    .unwrap();
    let first_tag = first_idx(k, |e| matches!(e, EventKind::TagCreatedLocal { .. })).unwrap();
    assert!(
        pub_done < first_tag,
        "the tag was created before publish-all closed"
    );

    // The command log confirms it at the process level: both builds precede both
    // publishes (no target ran its publish while another was still building).
    let calls = cmd.calls();
    let last_build = calls
        .iter()
        .rposition(|c| c.contains("package") || c.contains("pack"))
        .unwrap();
    let first_publish = calls.iter().position(|c| c.contains("publish")).unwrap();
    assert!(
        last_build < first_publish,
        "a publish ran before every build finished: {calls:?}"
    );
}

// ── Tag-once, coordinator-owned ──────────────────────────────────────────────

#[test]
fn tags_exactly_once_after_all_publishes_and_completes_the_run() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new();
    let reg = FakeRegistry;
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = new_journal(&store, &clock, &idgen);

    execute(&mut journal, &two_target_plan(), &ctx, &tagger, &mut sink).unwrap();

    // Exactly one tag, driven through its three steps in order.
    assert_eq!(
        tagger.calls(),
        vec!["create:v1.2.3@deadbeef", "push:v1.2.3", "release:v1.2.3"]
    );

    let state = journal.state();
    assert_eq!(
        state.status,
        RunStatus::Completed,
        "tag-ok completes the run"
    );
    assert_eq!(state.published.len(), 2);
    let tag = state.tags.get("v1.2.3").expect("tag recorded");
    assert!(tag.created_local && tag.pushed_remote && tag.github_release);
    assert_eq!(
        tag.github_release_url.as_deref(),
        Some("https://github.com/x/y/releases/v1.2.3")
    );
    // Every barrier completed Ok.
    for phase in [Phase::DryRun, Phase::Build, Phase::Publish, Phase::Tag] {
        assert!(
            state
                .phases
                .iter()
                .any(|r| r.phase == phase && r.outcome == PhaseOutcome::Ok),
            "{phase:?} did not complete Ok"
        );
    }
}

// ── Failure stops and journals, with no rollback ─────────────────────────────

#[test]
fn publish_failure_stops_journals_and_does_not_roll_back() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    // cargo (rust) is first and publishes fine; npm's publish fails. `npm pack`
    // (build) must still succeed, so we key the failure on the exact publish line.
    let cmd = FakeCmd::failing_on("npm publish");
    let reg = FakeRegistry;
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = new_journal(&store, &clock, &idgen);

    let err = execute(&mut journal, &two_target_plan(), &ctx, &tagger, &mut sink).unwrap_err();
    match err {
        CutError::PhaseFailed { phase, target, .. } => {
            assert_eq!(phase, Phase::Publish);
            assert_eq!(target.as_deref(), Some("node"));
        }
        other => panic!("expected a publish PhaseFailed, got {other:?}"),
    }

    let state = journal.state();
    // No rollback: the already-published rust target's receipt is intact.
    assert!(
        state.published.contains_key("rust"),
        "landed publish was rolled back"
    );
    assert!(
        !state.published.contains_key("node"),
        "failed publish recorded a receipt"
    );
    // The failed barrier is journalled; the run is not completed and never tagged.
    assert!(state
        .phases
        .iter()
        .any(|r| r.phase == Phase::Publish && r.outcome == PhaseOutcome::Failed));
    assert_eq!(state.status, RunStatus::InProgress);
    assert!(
        state.tags.is_empty(),
        "a tag was created despite a failed publish"
    );
    assert!(
        tagger.calls().is_empty(),
        "the tagger ran despite a failed publish"
    );
}

#[test]
fn a_failed_build_never_publishes_or_tags() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let reg = FakeRegistry;
    let root = PathBuf::from("/repo");
    let tagger = FakeTagger::new();
    let mut sink = NullSink;
    let mut journal = new_journal(&store, &clock, &idgen);

    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.0.0".into(),
        targets: vec![plan_target(
            Ecosystem::Rust,
            Registry::CratesIo,
            Adapter::CargoPublish,
        )],
        phases: PlanPhase::SEQUENCE.to_vec(),
    };
    // `dry_run` shells out to nothing, so fail the build step (`cargo package`)
    // and assert publish never runs and no tag is created.
    let cmd = FakeCmd::failing_on("package");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let err = execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap_err();
    assert!(matches!(
        err,
        CutError::PhaseFailed {
            phase: Phase::Build,
            ..
        }
    ));
    assert!(
        !cmd.calls().iter().any(|c| c.contains("publish")),
        "published after a failed build"
    );
    assert!(journal.state().published.is_empty());
    assert!(journal.state().tags.is_empty());
    assert!(tagger.calls().is_empty());
}

#[test]
fn a_tag_push_failure_journals_the_local_tag_and_stops() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new();
    let reg = FakeRegistry;
    let root = PathBuf::from("/repo");
    // The tag is created locally, then the push fails: no rollback, and the
    // local-tag fact is journalled so a resume retries only the push.
    let tagger = FakeTagger::failing("push");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = new_journal(&store, &clock, &idgen);

    let err = execute(&mut journal, &two_target_plan(), &ctx, &tagger, &mut sink).unwrap_err();
    match err {
        CutError::PhaseFailed { phase, target, .. } => {
            assert_eq!(phase, Phase::Tag);
            assert_eq!(target, None, "a coordinator tag step has no target");
        }
        other => panic!("expected a tag PhaseFailed, got {other:?}"),
    }

    let state = journal.state();
    // All publishes landed and stay landed; the local tag is recorded, push is not.
    assert_eq!(state.published.len(), 2);
    let tag = state.tags.get("v1.2.3").expect("local tag recorded");
    assert!(tag.created_local, "local tag fact was not journalled");
    assert!(!tag.pushed_remote, "push recorded despite failing");
    assert!(!tag.github_release);
    assert_eq!(state.status, RunStatus::InProgress);
    assert!(state
        .phases
        .iter()
        .any(|r| r.phase == Phase::Tag && r.outcome == PhaseOutcome::Failed));
    // The GitHub Release step was never attempted after the push failed.
    assert_eq!(
        tagger.calls(),
        vec!["create:v1.2.3@deadbeef", "push:v1.2.3"]
    );
}

// ── Resume-ready journal state (idempotent re-entry) ─────────────────────────

#[test]
fn re_execution_resumes_without_re_publishing_landed_targets() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let reg = FakeRegistry;
    let root = PathBuf::from("/repo");
    let tagger = FakeTagger::new();

    // First attempt: npm publish fails after rust has published.
    {
        let cmd = FakeCmd::failing_on("npm publish");
        let ctx = EffectCtx {
            runner: &cmd,
            clock: &clock,
            registry: &reg,
            repo_root: &root,
            artifacts: &EMPTY_ARTIFACTS,
        };
        let mut sink = NullSink;
        let mut journal = new_journal(&store, &clock, &idgen);
        execute(&mut journal, &two_target_plan(), &ctx, &tagger, &mut sink).unwrap_err();
        assert!(journal.state().published.contains_key("rust"));
        assert!(!journal.state().published.contains_key("node"));
    } // journal drops → releases the lock

    // Second attempt (resume): everything succeeds now. Reopen the journal from
    // the durable log and re-run the coordinator with a fresh runner.
    let cmd = FakeCmd::new();
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = Journal::open(&store, &clock, paths(), "RUN01").unwrap();
    execute(&mut journal, &two_target_plan(), &ctx, &tagger, &mut sink).unwrap();

    // The already-landed rust publish was NOT re-run…
    assert!(
        !cmd.calls().iter().any(|c| c == "cargo publish -p tool"),
        "resume re-published an already-landed target: {:?}",
        cmd.calls()
    );
    // …dry-run/build (completed Ok before the failure) were skipped whole…
    assert!(
        !cmd.calls().iter().any(|c| c.contains("package")),
        "resume re-ran a completed build phase: {:?}",
        cmd.calls()
    );
    // …only npm's publish was resumed, and the run completed + tagged.
    assert!(cmd.calls().iter().any(|c| c == "npm publish"));
    let state = journal.state();
    assert_eq!(state.status, RunStatus::Completed);
    assert!(state.published.contains_key("rust") && state.published.contains_key("node"));
    assert_eq!(
        tagger.calls(),
        vec!["create:v1.2.3@deadbeef", "push:v1.2.3", "release:v1.2.3"]
    );
}

// ── Artifact threading: build outputs + source tarball reach publish ──────────

/// A journal for a rust + binary/homebrew skeleton cut (both distribution
/// adapters share the `binary` ecosystem journal id).
fn skeleton_journal<'a>(
    store: &'a FakeStore,
    clock: &'a FakeClock,
    idgen: &'a FakeIdGen,
    version: &str,
) -> Journal<'a> {
    Journal::create(
        store,
        clock,
        idgen,
        paths(),
        "plan-test".into(),
        version.to_string(),
        vec!["rust".into(), "binary".into()],
    )
    .unwrap()
}

#[test]
fn threads_build_assets_into_binary_publish() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new();
    let reg = FakeRegistry;
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = skeleton_journal(&store, &clock, &idgen, "1.2.3");

    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.2.3".into(),
        targets: vec![
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublish),
            plan_target(Ecosystem::Binary, Registry::GhReleases, Adapter::Manual),
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
    };
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    // The build phase's artifacts are aggregated and reach the binary adapter's
    // publish command (which has no build output of its own). This asserts the
    // *plumbing* — that a build output threads through to the upload set; WHICH
    // ecosystems' outputs the binary adapter should actually attach to a GitHub
    // Release (cargo's `.crate` is a registry artifact, not a distributable) is a
    // selection policy owned by `adapter-skeletons-finish`, not this seam.
    assert!(
        cmd.calls()
            .iter()
            .any(|c| c == "gh release upload v1.2.3 --clobber -- tool-1.2.3.crate"),
        "binary publish did not receive the threaded build asset: {:?}",
        cmd.calls()
    );
}

#[test]
fn threads_source_tarball_url_into_homebrew_publish() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::with_origin("git@github.com:o/r.git");
    let reg = FakeRegistry;
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = skeleton_journal(&store, &clock, &idgen, "1.0.0");

    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.0.0".into(),
        targets: vec![
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublish),
            plan_target(Ecosystem::Binary, Registry::Homebrew, Adapter::HomebrewTap),
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
    };
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    // The `origin` remote resolves to the deterministic GitHub tag-archive URL,
    // threaded into the formula bump's `--url` (the sha256 lands with the finished
    // homebrew body — it is `None` at this layer).
    assert!(
        cmd.calls().iter().any(|c| c
            == "brew bump-formula-pr --url \
                https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz -- tool"),
        "homebrew publish did not receive the threaded tarball URL: {:?}",
        cmd.calls()
    );
}

#[test]
fn no_source_tarball_lookup_without_a_homebrew_target() {
    // A rust+binary cut needs no source tarball, so the coordinator never shells
    // out to `git remote get-url origin` (the gate keeps unrelated cuts clean).
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::with_origin("git@github.com:o/r.git");
    let reg = FakeRegistry;
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = skeleton_journal(&store, &clock, &idgen, "1.2.3");

    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.2.3".into(),
        targets: vec![
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublish),
            plan_target(Ecosystem::Binary, Registry::GhReleases, Adapter::Manual),
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
    };
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    assert!(
        !cmd.calls().iter().any(|c| c == "git remote get-url origin"),
        "resolved a source tarball for a cut with no homebrew target: {:?}",
        cmd.calls()
    );
}

#[test]
fn threads_no_assets_when_build_phase_is_resumed() {
    // Pins the documented resume boundary: a resume that re-enters after a
    // completed build phase re-gathers no assets (they were gathered on the run
    // that first built), so the binary adapter's publish sees an empty upload set.
    // The two distribution adapters do not perform a real upload yet — making
    // artifacts survive resume is `adapter-skeletons-finish`. Guarding this now
    // would just move a SKELETON's incompleteness earlier; here we lock the
    // behavior so the follow-up (which journals artifacts) has a regression anchor.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let reg = FakeRegistry;
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");

    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.2.3".into(),
        targets: vec![
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublish),
            plan_target(Ecosystem::Binary, Registry::GhReleases, Adapter::Manual),
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
    };

    // First attempt: the binary publish fails after build completed, leaving the
    // build phase journalled Ok and the run resumable.
    {
        let cmd = FakeCmd::failing_on("gh release upload");
        let ctx = EffectCtx {
            runner: &cmd,
            clock: &clock,
            registry: &reg,
            repo_root: &root,
            artifacts: &EMPTY_ARTIFACTS,
        };
        let mut sink = NullSink;
        let mut journal = skeleton_journal(&store, &clock, &idgen, "1.2.3");
        execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap_err();
        // The first attempt DID gather the asset (build ran) and passed it through.
        assert!(cmd.calls().iter().any(|c| c.contains("tool-1.2.3.crate")));
    }

    // Resume: build is skipped whole (completed Ok), so `assets` is empty and the
    // binary upload carries no asset path.
    let cmd = FakeCmd::new();
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = Journal::open(&store, &clock, paths(), "RUN01").unwrap();
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    assert!(
        !cmd.calls().iter().any(|c| c.contains("package")),
        "resume re-ran the completed build: {:?}",
        cmd.calls()
    );
    assert!(
        cmd.calls()
            .iter()
            .any(|c| c == "gh release upload v1.2.3 --clobber --"),
        "resumed binary upload should carry no assets (documented boundary): {:?}",
        cmd.calls()
    );
}

// ── Plan validation (before any external action) ─────────────────────────────

#[test]
fn refuses_a_target_with_no_resolved_package() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new();
    let reg = FakeRegistry;
    let root = PathBuf::from("/repo");
    let tagger = FakeTagger::new();
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = NullSink;
    let mut journal = new_journal(&store, &clock, &idgen);

    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.0.0".into(),
        targets: vec![PlanTarget {
            ecosystem: Ecosystem::Rust,
            package: None, // unresolved → not executable
            registry: Registry::CratesIo,
            adapter: Adapter::CargoPublish,
        }],
        phases: PlanPhase::SEQUENCE.to_vec(),
    };
    let err = execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap_err();
    assert!(matches!(err, CutError::Plan(_)));
    // Refused before any command or tag ran.
    assert!(cmd.calls().is_empty());
    assert!(tagger.calls().is_empty());
}
