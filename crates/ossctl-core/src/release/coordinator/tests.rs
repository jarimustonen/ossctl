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

/// The canned sha256 the fake `shasum` reports for the post-tag source tarball —
/// a fixed 64-hex digest so a homebrew finalize threads a deterministic `--sha256`.
const CANNED_SHA256: &str = "1111111111111111111111111111111111111111111111111111111111111111";

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
            fail_contains: Some(substr.to_string()),
            ..Self::new()
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
        // Serve the origin remote so `resolve_repo_slug` can parse a GitHub slug.
        if let Some(origin) = &self.origin {
            if program == "git" && args == ["remote", "get-url", "origin"] {
                return Ok(CommandOutput {
                    status: Some(0),
                    stdout: origin.clone(),
                    stderr: String::new(),
                });
            }
        }
        // Serve a single-crate (`tool`) workspace graph for the cargo adapter's
        // `cargo metadata` probe, so the rust target resolves to one publishable
        // member (these coordinator tests exercise the phase machinery, not the
        // multi-crate publish order — that lives in the adapter's own tests).
        if program == "cargo" && args.contains(&"metadata") {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: r#"{"packages":[{"name":"tool","version":"1.0.0","id":"tool 1.0.0","dependencies":[],"publish":null}],"workspace_members":["tool 1.0.0"]}"#.to_string(),
                stderr: String::new(),
            });
        }
        // Serve the post-tag dist phase's source-tarball hash: `shasum -a 256 <tmp>`
        // prints `<hex>  <file>`. A fixed canned digest lets the homebrew finalize
        // thread a real `--sha256` deterministically (the fake `curl` that "wrote"
        // the temp file succeeds by default, and the coordinator never reads it).
        if program == "shasum" || program == "sha256sum" {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: format!("{CANNED_SHA256}  /tmp/ossctl-src-tarball.tar.gz"),
                stderr: String::new(),
            });
        }
        // Serve the node adapter's `npm pack --json` probe the packed tarball name,
        // so the node build phase can identify its artifact (the build fails hard on
        // unparseable output, so an empty default would abort the phase machinery
        // these tests exercise).
        if program == "npm" && args.contains(&"pack") {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: r#"[{"filename":"tool-1.0.0.tgz"}]"#.to_string(),
                stderr: String::new(),
            });
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
        homebrew_tap: None,
        license: None,
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
        homebrew_tap: None,
        license: None,
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
        homebrew_tap: None,
        license: None,
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
        homebrew_tap: None,
        license: None,
    };
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    // The homebrew formula is finalized in the POST-TAG dist phase, now that the tag
    // archive exists: the coordinator resolves the `origin` slug to the deterministic
    // tag-archive URL, fetches it (`curl`), hashes it (`shasum`), and threads the
    // REAL `--sha256` into the formula bump — no draft placeholder, no `sha256: None`.
    let bump = format!(
        "brew bump-formula-pr --url \
         https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz --sha256 {CANNED_SHA256} -- tool"
    );
    assert!(
        cmd.calls().contains(&bump),
        "homebrew publish did not receive the tarball URL + real sha256: {:?}",
        cmd.calls()
    );
    // The digest is computed from the pushed tag archive (curl + shasum), never from
    // a local `git archive` (whose bytes diverge from GitHub's served tarball).
    assert!(
        cmd.calls()
            .iter()
            .any(|c| c.starts_with("curl -sSfL -o ") && c.ends_with(".tar.gz")),
        "the tag archive was not fetched before hashing: {:?}",
        cmd.calls()
    );
    assert!(
        cmd.calls()
            .iter()
            .any(|c| c.starts_with("sha256sum ") || c.starts_with("shasum -a 256 ")),
        "the fetched tag archive was not hashed: {:?}",
        cmd.calls()
    );
    assert!(
        !cmd.calls().iter().any(|c| c.contains("git archive")),
        "a local source archive was hashed despite the wrong-bytes hazard: {:?}",
        cmd.calls()
    );
    // The homebrew target is recorded as published (in the dist phase), and the run
    // completed via the dist barrier.
    assert!(journal.state().published.contains_key("binary"));
    assert_eq!(journal.state().status, RunStatus::Completed);
}

#[test]
fn no_slug_lookup_without_a_github_distribution_target() {
    // A rust+python cut carries no GitHub-backed distribution target (binary or
    // homebrew), so the coordinator never shells out to `git remote get-url origin`
    // — the gate keeps unrelated cuts clean.
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
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-test".into(),
        "1.2.3".into(),
        vec!["rust".into(), "python".into()],
    )
    .unwrap();

    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.2.3".into(),
        targets: vec![
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublish),
            plan_target(Ecosystem::Python, Registry::Pypi, Adapter::Twine),
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
        homebrew_tap: None,
        license: None,
    };
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    assert!(
        !cmd.calls().iter().any(|c| c == "git remote get-url origin"),
        "resolved a slug for a cut with no GitHub distribution target: {:?}",
        cmd.calls()
    );
}

#[test]
fn threads_repo_slug_into_binary_receipt() {
    // A rust+binary cut resolves the `origin` slug and records the GitHub-Release
    // page URL on the binary target's receipt (persisted as the journal receipt's
    // `registry_url`). No homebrew target ⇒ no source-archive hashing.
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
        homebrew_tap: None,
        license: None,
    };
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    // The upload is pinned to the resolved slug with `--repo`.
    assert!(
        cmd.calls()
            .iter()
            .any(|c| c.starts_with("gh release upload v1.2.3 --repo o/r --clobber --")),
        "binary upload was not pinned to the resolved slug: {:?}",
        cmd.calls()
    );
    let receipt = journal
        .state()
        .published
        .get("binary")
        .expect("binary target published");
    assert_eq!(
        receipt.registry_url.as_deref(),
        Some("https://github.com/o/r/releases/tag/v1.2.3"),
        "binary receipt did not record the release URL from the threaded slug"
    );
    // No homebrew target ⇒ the source archive is never hashed.
    assert!(
        !cmd.calls().iter().any(|c| c.contains("git archive")),
        "hashed a source archive for a cut with no homebrew target: {:?}",
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
        homebrew_tap: None,
        license: None,
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

// ── Multiple targets in one ecosystem (dep-ordered) ──────────────────────────

/// A crates.io world shared by the command runner and the registry query: a
/// `cargo publish -p <crate>` (a real publish, not `--dry-run`) marks the crate
/// published, and the registry then reports it. Lets a multi-target rust cut
/// exercise the real is-published/index-wait skip logic the cargo adapter runs
/// between dependent crates.
#[derive(Clone, Default)]
struct CratesWorld {
    published: Rc<RefCell<HashSet<String>>>,
}

/// `git`-less command runner over a two-crate workspace (`ossctl-core` ← `ossctl`)
/// that records publishes into the shared [`CratesWorld`].
struct WorkspaceCmd {
    world: CratesWorld,
    calls: RefCell<Vec<String>>,
}
impl WorkspaceCmd {
    fn new(world: CratesWorld) -> Self {
        Self {
            world,
            calls: RefCell::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}
impl CommandRunner for WorkspaceCmd {
    fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        // Serve the `origin` remote so a homebrew target's cut resolves a slug.
        if program == "git" && args == ["remote", "get-url", "origin"] {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: "git@github.com:jarimustonen/ossctl.git".to_string(),
                stderr: String::new(),
            });
        }
        // Serve the post-tag source-tarball hash for the dist phase.
        if program == "shasum" || program == "sha256sum" {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: format!("{CANNED_SHA256}  /tmp/ossctl-src-tarball.tar.gz"),
                stderr: String::new(),
            });
        }
        if program == "cargo" && args.contains(&"metadata") {
            // Two publishable members: `ossctl` depends on `ossctl-core`.
            return Ok(CommandOutput {
                status: Some(0),
                stdout: r#"{"packages":[
                    {"name":"ossctl-core","version":"1.2.3","id":"ossctl-core 1.2.3","dependencies":[],"publish":null},
                    {"name":"ossctl","version":"1.2.3","id":"ossctl 1.2.3","dependencies":[{"name":"ossctl-core"}],"publish":null}
                ],"workspace_members":["ossctl-core 1.2.3","ossctl 1.2.3"]}"#.to_string(),
                stderr: String::new(),
            });
        }
        // A real publish (`cargo publish -p X`, no `--dry-run`) lands the crate.
        if program == "cargo" && args.first() == Some(&"publish") && !args.contains(&"--dry-run") {
            if let Some(pos) = args.iter().position(|a| *a == "-p") {
                if let Some(pkg) = args.get(pos + 1) {
                    self.world.published.borrow_mut().insert((*pkg).to_string());
                }
            }
        }
        Ok(CommandOutput {
            status: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

/// Registry view over the shared [`CratesWorld`]: reports `1.2.3` once a crate has
/// been published.
struct WorldRegistry {
    world: CratesWorld,
}
impl RegistryQuery for WorldRegistry {
    fn published_versions(&self, _ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        Ok(if self.world.published.borrow().contains(package) {
            vec!["1.2.3".to_string()]
        } else {
            Vec::new()
        })
    }
}

fn crates_target(package: &str) -> PlanTarget {
    PlanTarget {
        ecosystem: Ecosystem::Rust,
        package: Some(package.to_string()),
        registry: Registry::CratesIo,
        adapter: Adapter::CargoPublish,
    }
}

#[test]
fn two_crates_io_targets_in_one_ecosystem_cut_in_dependency_order() {
    // The `release-cut-multi-target-ecosystem` regression: a contract with two
    // crates.io targets in the `rust` ecosystem (`ossctl-core`, then `ossctl`) is
    // no longer rejected as `invalid_plan`; it cuts, keying each target by a
    // distinct journal id and publishing the dependency before its dependent.
    let world = CratesWorld::default();
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = WorkspaceCmd::new(world.clone());
    let reg = WorldRegistry {
        world: world.clone(),
    };
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };

    let plan = ReleasePlan {
        plan_id: "plan-multi".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.2.3".into(),
        targets: vec![crates_target("ossctl-core"), crates_target("ossctl")],
        phases: PlanPhase::SEQUENCE.to_vec(),
        homebrew_tap: None,
        license: None,
    };

    // The plan preflights (this is the check that used to hard-fail `invalid_plan`)
    // and its two targets carry distinct, dependency-ordered journal ids.
    validate_plan(&plan).expect("two same-ecosystem targets must be a valid plan");
    let ids = crate::release::journal_target_ids(&plan.targets);
    assert_eq!(ids, vec!["rust:ossctl-core", "rust:ossctl"]);

    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-multi".into(),
        "1.2.3".into(),
        ids.clone(),
    )
    .unwrap();
    let mut sink = NullSink;
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    let state = journal.state();
    assert_eq!(state.status, RunStatus::Completed);
    // Both targets published under their distinct ids — no ecosystem collision.
    assert!(state.published.contains_key("rust:ossctl-core"));
    assert!(state.published.contains_key("rust:ossctl"));
    assert_eq!(state.published.len(), 2);

    // Dependency order at the process level: `ossctl-core` publishes before
    // `ossctl`, and exactly once (ADR-0004: each target publishes only its own
    // crate — `ossctl`'s target waits for `ossctl-core` to index, never re-publishes
    // it).
    let calls = cmd.calls();
    let core_pub = calls
        .iter()
        .position(|c| c == "cargo publish -p ossctl-core")
        .expect("ossctl-core was published");
    let cli_pub = calls
        .iter()
        .position(|c| c == "cargo publish -p ossctl")
        .expect("ossctl was published");
    assert!(
        core_pub < cli_pub,
        "dependency published after its dependent: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "cargo publish -p ossctl-core")
            .count(),
        1,
        "the dependency crate was published more than once: {calls:?}"
    );
}

/// A registry over the shared [`CratesWorld`] that reports a *published* crate as
/// index-visible only after `lag` polls — the crates.io publish→index lag window
/// in which the old closure-per-target model re-ran `cargo publish` for a shared
/// dependency (the double-publish bug this test guards against).
struct LaggingWorldRegistry {
    world: CratesWorld,
    lag: u32,
    polls: RefCell<HashMap<String, u32>>,
}
impl RegistryQuery for LaggingWorldRegistry {
    fn published_versions(&self, _ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        if !self.world.published.borrow().contains(package) {
            return Ok(Vec::new());
        }
        let mut polls = self.polls.borrow_mut();
        let seen = polls.entry(package.to_string()).or_insert(0);
        if *seen >= self.lag {
            Ok(vec!["1.2.3".to_string()])
        } else {
            *seen += 1;
            Ok(Vec::new())
        }
    }
}

/// A clock whose `sleep` advances virtual time (never a real thread sleep) so the
/// index-wait loop terminates instantly under test; `now_unix` still advances so
/// the bounded-wait timeout math is exercised.
struct SleepAdvancingClock(Cell<u64>);
impl Clock for SleepAdvancingClock {
    fn now_unix(&self) -> u64 {
        let t = self.0.get();
        self.0.set(t + 1);
        t
    }
    fn sleep(&self, dur: std::time::Duration) {
        self.0.set(self.0.get() + dur.as_secs().max(1));
    }
}

#[test]
fn two_targets_do_not_double_publish_a_shared_dependency_under_index_lag() {
    // The core `cargo-adapter-multitarget-double-publish` regression AT THE
    // COORDINATOR LEVEL: two crates.io targets `ossctl-core` then `ossctl` cut in
    // dependency order, where `ossctl-core` stays index-lagged for several polls
    // after it publishes. The old model re-ran `cargo publish -p ossctl-core` from
    // `ossctl`'s closure during that lag (a partial-publish trap); ADR-0004's
    // one-target-one-publish model must publish each crate EXACTLY ONCE.
    let world = CratesWorld::default();
    let store = FakeStore::default();
    let clock = SleepAdvancingClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = WorkspaceCmd::new(world.clone());
    let reg = LaggingWorldRegistry {
        world: world.clone(),
        lag: 3,
        polls: RefCell::new(HashMap::new()),
    };
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };

    let plan = ReleasePlan {
        plan_id: "plan-lag".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.2.3".into(),
        targets: vec![crates_target("ossctl-core"), crates_target("ossctl")],
        phases: PlanPhase::SEQUENCE.to_vec(),
        homebrew_tap: None,
        license: None,
    };
    let ids = crate::release::journal_target_ids(&plan.targets);
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-lag".into(),
        "1.2.3".into(),
        ids,
    )
    .unwrap();
    let mut sink = NullSink;
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    assert_eq!(journal.state().status, RunStatus::Completed);
    let calls = cmd.calls();
    // The dependency was published exactly once despite the index lag before the
    // dependent's target ran — never a second `cargo publish -p ossctl-core`.
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "cargo publish -p ossctl-core")
            .count(),
        1,
        "the shared dependency was published more than once under lag: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "cargo publish -p ossctl")
            .count(),
        1
    );
    // Dependency before dependent.
    let core = calls
        .iter()
        .position(|c| c == "cargo publish -p ossctl-core")
        .unwrap();
    let cli = calls
        .iter()
        .position(|c| c == "cargo publish -p ossctl")
        .unwrap();
    assert!(
        core < cli,
        "dependency published after its dependent: {calls:?}"
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
        homebrew_tap: None,
        license: None,
    };
    let err = execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap_err();
    assert!(matches!(err, CutError::Plan(_)));
    // Refused before any command or tag ran.
    assert!(cmd.calls().is_empty());
    assert!(tagger.calls().is_empty());
}

// ── CI-delegated skip + post-tag homebrew (release-engine-cut-cargo-dist-flow) ─

#[test]
fn ci_delegated_target_is_skipped_journaled_not_failed() {
    // A cargo-dist (gh-releases) target is CI-delegated: its binaries are produced
    // by the tag-triggered release workflow, not the engine. The coordinator must
    // journal it `target_delegated` and SKIP it in publish — never publish it (its
    // `publish` returns Unsupported), never count it a failure. The crates.io target
    // publishes, the tag runs, and the run completes.
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

    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.0.0".into(),
        targets: vec![
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublish),
            PlanTarget {
                ecosystem: Ecosystem::Rust,
                package: Some("tool".into()),
                registry: Registry::GhReleases,
                adapter: Adapter::CargoDist,
            },
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
        homebrew_tap: None,
        license: None,
    };
    let ids = crate::release::journal_target_ids(&plan.targets);
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "p".into(),
        "1.0.0".into(),
        ids.clone(),
    )
    .unwrap();
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    let state = journal.state();
    assert_eq!(state.status, RunStatus::Completed);
    // The crates.io target published; the cargo-dist target did NOT.
    assert!(state.published.contains_key(&ids[0]));
    assert!(!state.published.contains_key(&ids[1]));
    // …and the cargo-dist target is journalled delegated (neither published/failed).
    assert!(state.delegated.contains(&ids[1]));
    // Every barrier — including publish — completed Ok (delegation is not a failure).
    for phase in [
        Phase::DryRun,
        Phase::Build,
        Phase::Publish,
        Phase::Tag,
        Phase::Dist,
    ] {
        assert!(
            state
                .phases
                .iter()
                .any(|r| r.phase == phase && r.outcome == PhaseOutcome::Ok),
            "{phase:?} did not complete Ok"
        );
    }
    // No cargo-dist publish/upload command ran (its publish was never called).
    assert!(
        !cmd.calls().iter().any(|c| c == "dist publish"),
        "a delegated cargo-dist target was published: {:?}",
        cmd.calls()
    );
    // The delegation is a streamed event carrying the adapter identity, recorded in
    // the publish phase (before the tag).
    let k = &sink.kinds;
    let delegated_idx = first_idx(
        k,
        |e| matches!(e, EventKind::TargetDelegated { adapter, .. } if adapter == "cargo-dist"),
    )
    .expect("a target_delegated event was streamed");
    let first_tag = first_idx(k, |e| matches!(e, EventKind::TagCreatedLocal { .. })).unwrap();
    assert!(delegated_idx < first_tag, "delegation must precede tagging");
}

// A comprehensive end-to-end acceptance across all four target classes; splitting
// it would fragment one coherent scenario, so the line ceiling is waived here.
#[allow(clippy::too_many_lines)]
#[test]
fn ossctl_like_contract_cuts_end_to_end_across_target_classes() {
    // The `release-engine-cut-cargo-dist-flow` acceptance: ossctl's own contract —
    // two crates.io targets (ossctl-core → ossctl), a gh-releases cargo-dist target
    // (CI-delegated), and a homebrew-tap target (post-tag) — cuts end to end. The
    // crates.io crates publish in dependency order, cargo-dist is skipped/delegated,
    // the tag runs, and homebrew is finalized POST-TAG with a real sha256.
    let world = CratesWorld::default();
    let store = FakeStore::default();
    let clock = SleepAdvancingClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = WorkspaceCmd::new(world.clone());
    let reg = WorldRegistry {
        world: world.clone(),
    };
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };

    let ossctl_target = |registry, adapter| PlanTarget {
        ecosystem: Ecosystem::Rust,
        package: Some("ossctl".into()),
        registry,
        adapter,
    };
    let plan = ReleasePlan {
        plan_id: "plan-ossctl".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.2.3".into(),
        targets: vec![
            crates_target("ossctl-core"),
            crates_target("ossctl"),
            ossctl_target(Registry::GhReleases, Adapter::CargoDist),
            ossctl_target(Registry::Homebrew, Adapter::HomebrewTap),
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
        homebrew_tap: Some("jarimustonen/homebrew-ossctl".into()),
        license: Some("MIT".into()),
    };
    let ids = crate::release::journal_target_ids(&plan.targets);
    // ids: [ossctl-core@crates.io, ossctl@crates.io, ossctl@gh-releases, ossctl@homebrew]
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-ossctl".into(),
        "1.2.3".into(),
        ids.clone(),
    )
    .unwrap();
    let mut sink = RecordingSink::default();
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    let state = journal.state();
    assert_eq!(state.status, RunStatus::Completed);
    let calls = cmd.calls();
    // Both crates.io crates published exactly once, dependency before dependent.
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "cargo publish -p ossctl-core")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "cargo publish -p ossctl")
            .count(),
        1
    );
    let core = calls
        .iter()
        .position(|c| c == "cargo publish -p ossctl-core")
        .unwrap();
    let cli = calls
        .iter()
        .position(|c| c == "cargo publish -p ossctl")
        .unwrap();
    assert!(
        core < cli,
        "dependency published after its dependent: {calls:?}"
    );
    // crates.io targets published; cargo-dist delegated (skipped); homebrew finalized.
    assert!(state.published.contains_key(&ids[0]) && state.published.contains_key(&ids[1]));
    assert!(state.delegated.contains(&ids[2]) && !state.published.contains_key(&ids[2]));
    assert!(
        state.published.contains_key(&ids[3]),
        "homebrew was not finalized in the dist phase"
    );
    // The homebrew bump carried the tag-archive URL AND the real post-tag sha256.
    let bump = format!(
        "brew bump-formula-pr --url \
         https://github.com/jarimustonen/ossctl/archive/refs/tags/v1.2.3.tar.gz \
         --sha256 {CANNED_SHA256} -- ossctl"
    );
    assert!(
        calls.contains(&bump),
        "homebrew was not finalized with a real sha256: {calls:?}"
    );

    // Barrier ordering at the event level: publish closes → tag → dist, and the
    // homebrew finalize (a published event in the dist phase) follows the tag.
    let k = &sink.kinds;
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
    let tag_local = first_idx(k, |e| matches!(e, EventKind::TagCreatedLocal { .. })).unwrap();
    let dist_entered = first_idx(k, |e| {
        matches!(e, EventKind::PhaseEntered { phase: Phase::Dist })
    })
    .unwrap();
    assert!(
        pub_done < tag_local && tag_local < dist_entered,
        "phase order publish → tag → dist was violated"
    );
}
