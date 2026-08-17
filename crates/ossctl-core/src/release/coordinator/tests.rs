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
use tempfile::TempDir;

use super::*;
use crate::contract::schema::{Adapter, Ecosystem, Registry};
use crate::ports::{
    Clock, CommandOutput, CommandRunner, IdGen, JournalLock, JournalStore, RegistryQuery,
};
use crate::protocol::journal::{Phase, PhaseOutcome, RunStatus};
use crate::protocol::plan::{BumpLevel, BumpPlan, PinRewrite, PlanPhase, PlanTarget, ReleasePlan};
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
    /// The `cwd` each recorded call ran in — parallel to [`Self::calls`] (same
    /// index). Lets the clean-checkout tests assert an effect command ran against the
    /// sealed-commit worktree, not the live repo root (`release-cut-clean-checkout`).
    cwds: RefCell<Vec<PathBuf>>,
    /// When set, `git cat-file -e <sha>^{commit}` exits non-zero — modelling a sealed
    /// commit that is not present locally, so the clean checkout fails closed.
    cat_file_missing: bool,
    fail_contains: Option<String>,
    /// Canned `git remote get-url origin` stdout — lets a cut resolve a source
    /// tarball URL. `None` means "no origin" (empty stdout, as a bare repo).
    origin: Option<String>,
    /// Crates the runner has successfully `cargo publish`ed, keyed by
    /// `(ecosystem, package)` → published version. Shared with [`FakeRegistry`] via
    /// [`FakeCmd::registry`] so the self-visibility confirm
    /// (`cut-noop-self-visibility-check`) sees the crate the runner just published —
    /// the faithful "publish landed" model. A *failed* publish records nothing (so a
    /// no-op/failed upload correctly stays invisible).
    published: Rc<RefCell<HashMap<(String, String), String>>>,
    /// The version a successful `cargo publish` records for the crate — the plan
    /// version the confirm checks against (default `1.2.3`; override with
    /// [`FakeCmd::crate_version`] for a plan cut at a different version).
    publish_version: String,
    /// When set, `git worktree add --detach <dest> <sha>` copies this seed workspace
    /// tree into `<dest>` (a real directory), so the bump executor's real `std::fs`
    /// manifest/CHANGELOG edits run against real files in the sealed checkout.
    worktree_seed: Option<PathBuf>,
    /// The sha `git rev-parse HEAD` reports after the bump commit (the bump-commit sha
    /// the tag must point at), when a bump test set one.
    bump_commit: Option<String>,
}
impl FakeCmd {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            cwds: RefCell::new(Vec::new()),
            cat_file_missing: false,
            fail_contains: None,
            origin: None,
            published: Rc::new(RefCell::new(HashMap::new())),
            publish_version: "1.2.3".to_string(),
            worktree_seed: None,
            bump_commit: None,
        }
    }
    /// Seed the sealed checkout from `seed` and report `bump_commit` as the post-bump
    /// `git rev-parse HEAD` — the setup a coordinator-level `--bump` test needs.
    fn with_bump_checkout(mut self, seed: PathBuf, bump_commit: &str) -> Self {
        self.worktree_seed = Some(seed);
        self.bump_commit = Some(bump_commit.to_string());
        self
    }
    /// A runner whose `git cat-file` reports the sealed commit absent — the
    /// clean-checkout fail-closed path (`release-cut-clean-checkout`).
    fn missing_sealed_commit() -> Self {
        Self {
            cat_file_missing: true,
            ..Self::new()
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
    /// Record successful `cargo publish`es at this version (default `1.2.3`) — set it
    /// to a plan's version so the self-visibility confirm of that version passes.
    fn crate_version(mut self, version: &str) -> Self {
        self.publish_version = version.to_string();
        self
    }
    /// A [`FakeRegistry`] that reflects this runner's successful publishes — pass the
    /// SAME handle to the effect context so the confirm sees what the runner landed.
    fn registry(&self) -> FakeRegistry {
        FakeRegistry {
            published: Rc::clone(&self.published),
        }
    }
    /// Share `reg`'s published-crates map, so this runner's publishes land on `reg`
    /// (and, across a resume, a second runner sees what the first published). Used by
    /// multi-attempt resume tests where the registry outlives each per-attempt runner.
    fn sharing(mut self, reg: &FakeRegistry) -> Self {
        self.published = Rc::clone(&reg.published);
        self
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
    fn git_output(&self, args: &[&str]) -> Option<CommandOutput> {
        if args.first() == Some(&"cat-file") {
            return Some(CommandOutput {
                status: Some(i32::from(self.cat_file_missing)),
                stdout: String::new(),
                stderr: if self.cat_file_missing {
                    "fatal: git cat-file: could not get object info".into()
                } else {
                    String::new()
                },
            });
        }
        if args.first() == Some(&"worktree") && args.get(1) == Some(&"add") {
            if let Some(seed) = &self.worktree_seed {
                if let Some(dest) = args.iter().rev().nth(1) {
                    copy_tree(seed, Path::new(dest));
                }
            }
            return Some(CommandOutput {
                status: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        let stdout = match args {
            ["rev-parse", "HEAD"] => self.bump_commit.as_ref().map(|sha| format!("{sha}\n")),
            ["rev-parse", "--abbrev-ref", "HEAD"] => Some("main\n".to_string()),
            ["remote", "get-url", "origin"] => self.origin.clone(),
            _ => None,
        }?;
        Some(CommandOutput {
            status: Some(0),
            stdout,
            stderr: String::new(),
        })
    }
    fn homebrew_observation(&self, program: &str, args: &[&str]) -> Option<CommandOutput> {
        if program == "brew" && args.first() == Some(&"bump-formula-pr") {
            return Some(CommandOutput {
                status: Some(0),
                stdout: "https://github.com/Homebrew/homebrew-core/pull/1\n".to_string(),
                stderr: String::new(),
            });
        }
        (program == "curl"
            && args
                .last()
                .is_some_and(|url| url.contains("raw.githubusercontent.com")))
        .then(|| CommandOutput {
            status: Some(0),
            stdout: format!(
                "# Generated by ossctl; do not edit by hand (template-version: 2)\n\
                 class Tool < Formula\n  version \"{}\"\n  if OS.mac? && Hardware::CPU.arm?\n    url \"https://example/tool-aarch64-apple-darwin.tar.xz\"\n    sha256 \"{CANNED_SHA256}\"\n  end\nend\n",
                self.publish_version
            ),
            stderr: String::new(),
        })
    }
    /// The recorded `(rendered line, cwd)` pairs, for asserting *where* a command
    /// ran (the sealed-checkout tests).
    fn calls_with_cwd(&self) -> Vec<(String, PathBuf)> {
        self.calls
            .borrow()
            .iter()
            .cloned()
            .zip(self.cwds.borrow().iter().cloned())
            .collect()
    }
}
impl CommandRunner for FakeCmd {
    fn run(&self, program: &str, args: &[&str], cwd: &Path) -> io::Result<CommandOutput> {
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        self.cwds.borrow_mut().push(cwd.to_path_buf());
        if program == "git" {
            if let Some(output) = self.git_output(args) {
                return Ok(output);
            }
        }
        // Serve observed GitHub Release assets to the mandatory verify phase.
        if program == "gh" && args.starts_with(&["release", "view"]) && args.contains(&"--json") {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: r#"{"assets":[{"name":"tool-aarch64-apple-darwin.tar.xz"},{"name":"tool-x86_64-apple-darwin.tar.xz"},{"name":"tool-aarch64-unknown-linux-musl.tar.xz"},{"name":"tool-x86_64-unknown-linux-musl.tar.xz"},{"name":"ossctl-aarch64-apple-darwin.tar.xz"},{"name":"ossctl-x86_64-apple-darwin.tar.xz"},{"name":"ossctl-aarch64-unknown-linux-musl.tar.xz"},{"name":"ossctl-x86_64-unknown-linux-musl.tar.xz"}]}"#.into(),
                stderr: String::new(),
            });
        }
        // Serve the Homebrew publish receipt and a green post-publish formula
        // observation through the same read-only runner seam as production.
        if let Some(output) = self.homebrew_observation(program, args) {
            return Ok(output);
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
        // A successful twine upload is visible to the Python registry verification.
        if !fails && program == "twine" && args.first() == Some(&"upload") {
            self.published.borrow_mut().insert(
                ("python".to_string(), "tool".to_string()),
                self.publish_version.clone(),
            );
        }
        // A successful npm publish is visible to the node registry verification.
        if !fails && program == "npm" && args.first() == Some(&"publish") {
            self.published.borrow_mut().insert(
                ("node".to_string(), "tool".to_string()),
                self.publish_version.clone(),
            );
        }
        // A SUCCESSFUL `cargo publish -p <pkg>` (no `--dry-run`) lands the crate on the
        // shared registry, so the adapter's self-visibility confirm sees it. A FAILED
        // publish records nothing — a no-op/failed upload correctly stays invisible.
        if !fails
            && program == "cargo"
            && args.first() == Some(&"publish")
            && !args.contains(&"--dry-run")
        {
            if let Some(pos) = args.iter().position(|a| *a == "-p") {
                if let Some(pkg) = args.get(pos + 1) {
                    self.published.borrow_mut().insert(
                        ("rust".to_string(), (*pkg).to_string()),
                        self.publish_version.clone(),
                    );
                }
            }
        }
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
    fn sleep(&self, dur: std::time::Duration) {
        self.0.set(self.0.get().saturating_add(dur.as_secs()));
    }
}

struct FakeIdGen(String);
impl IdGen for FakeIdGen {
    fn new_id(&self) -> String {
        self.0.clone()
    }
}

/// Registry view over a [`FakeCmd`]'s successful publishes (via [`FakeCmd::registry`]):
/// reports `(ecosystem, package)` present at the recorded version once its
/// `cargo publish` landed, else absent. This gives the adapter's self-visibility
/// confirm (`cut-noop-self-visibility-check`) a faithful "did the publish land?" view.
struct FakeRegistry {
    published: Rc<RefCell<HashMap<(String, String), String>>>,
}
impl FakeRegistry {
    /// A standalone registry with no publishes reflected — for tests that never reach
    /// the publish phase (a dry-run/preflight failure, a plan-validation refusal), or
    /// that own a shared map handed to their runners via [`FakeCmd::sharing`].
    fn empty() -> Self {
        Self {
            published: Rc::new(RefCell::new(HashMap::new())),
        }
    }
}
impl RegistryQuery for FakeRegistry {
    fn http_get(&self, _url: &str) -> io::Result<(u16, Vec<u8>)> {
        let version = self
            .published
            .borrow()
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| "1.0.0".to_string());
        Ok((
            200,
            format!(
                "# Generated by ossctl; do not edit by hand (template-version: 2)\n\
                 class Tool < Formula\n  version \"{version}\"\n  if OS.mac? && Hardware::CPU.arm?\n    url \"https://example/tool-aarch64-apple-darwin.tar.xz\"\n    sha256 \"{CANNED_SHA256}\"\n  end\nend\n"
            )
            .into_bytes(),
        ))
    }

    fn published_versions(&self, ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        Ok(self
            .published
            .borrow()
            .get(&(ecosystem.to_string(), package.to_string()))
            .map(|v| vec![v.clone()])
            .unwrap_or_default())
    }
}

/// A cargo-dist-owned formula has no ossctl marker, but is otherwise complete.
struct UnmarkedFormulaRegistry;
impl RegistryQuery for UnmarkedFormulaRegistry {
    fn http_get(&self, _url: &str) -> io::Result<(u16, Vec<u8>)> {
        Ok((
            200,
            format!(
                "class Tool < Formula\n  version \"1.0.0\"\n  if OS.mac?\n    if Hardware::CPU.arm?\n      url \"https://example/tool-aarch64-apple-darwin.tar.xz\"\n      sha256 \"{CANNED_SHA256}\"\n    end\n  end\nend\n"
            )
            .into_bytes(),
        ))
    }

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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    }
}

#[test]
fn homebrew_fails_closed_when_a_servable_release_asset_is_missing() {
    let cmd = FakeCmd::failing_on("curl");
    let clock = FakeClock(Cell::new(0));
    let registry = FakeRegistry::empty();
    let root = Path::new("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &registry,
        repo_root: root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let plan = two_target_plan();
    let targets = resolve_target_plans(&plan).unwrap();
    let target_refs: Vec<&TargetPlan> = targets.iter().collect();
    let formula = HomebrewFormula {
        tap: Some("o/tap".into()),
        license: Some("MIT".into()),
        description: Some("Tool".into()),
        version: plan.version.clone(),
        platforms: vec![
            "aarch64-apple-darwin".into(),
            "x86_64-pc-windows-msvc".into(),
        ],
    };
    let err = fetch_homebrew_assets(&ctx, "o/r", &plan, &formula, &target_refs).unwrap_err();
    assert!(
        err.contains("not visible after") && err.contains("Refusing to write a source-build"),
        "{err}"
    );
    assert!(
        cmd.calls()
            .iter()
            .any(|call| call.contains("aarch64-apple-darwin")),
        "the servable platform was not requested: {:?}",
        cmd.calls()
    );
    assert!(
        !cmd.calls().iter().any(|call| call.contains("windows-msvc")),
        "the skipped Windows platform must not mask a missing servable asset: {:?}",
        cmd.calls()
    );
}

#[test]
fn homebrew_fetches_only_servable_platforms_from_the_full_distribution_set() {
    // This is ossctl's own distribution shape: cargo-dist also publishes Windows
    // for the GitHub Release, while Homebrew serves only the three Unix archives.
    let cmd = FakeCmd::new();
    let clock = FakeClock(Cell::new(0));
    let registry = FakeRegistry::empty();
    let root = Path::new("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &registry,
        repo_root: root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut plan = two_target_plan();
    plan.targets.push(plan_target(
        Ecosystem::Rust,
        Registry::Homebrew,
        Adapter::HomebrewTap,
    ));
    let targets = resolve_target_plans(&plan).unwrap();
    let target_refs: Vec<&TargetPlan> = targets
        .iter()
        .filter(|target| needs_post_tag(target))
        .collect();
    let formula = HomebrewFormula {
        tap: Some("o/tap".into()),
        license: Some("MIT".into()),
        description: Some("Tool".into()),
        version: plan.version.clone(),
        platforms: vec![
            "aarch64-apple-darwin".into(),
            "aarch64-unknown-linux-musl".into(),
            "x86_64-unknown-linux-musl".into(),
            "x86_64-pc-windows-msvc".into(),
        ],
    };

    let assets = fetch_homebrew_assets(&ctx, "o/r", &plan, &formula, &target_refs).unwrap();

    assert_eq!(
        assets
            .iter()
            .map(|asset| asset.triple.as_str())
            .collect::<Vec<_>>(),
        [
            "aarch64-apple-darwin",
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-musl",
        ]
    );
    assert!(
        !cmd.calls().iter().any(|call| call.contains("windows-msvc")),
        "Homebrew requested a Windows archive: {:?}",
        cmd.calls()
    );
}

#[test]
fn homebrew_rejects_a_distribution_with_no_servable_platforms() {
    let mut plan = two_target_plan();
    plan.targets.push(plan_target(
        Ecosystem::Rust,
        Registry::Homebrew,
        Adapter::HomebrewTap,
    ));
    plan.homebrew_platforms = vec!["x86_64-pc-windows-msvc".into()];

    let err = validate_plan(&plan).unwrap_err();

    assert!(
        matches!(err, CutError::Plan(ref message) if message.contains("no Homebrew-servable cargo-dist platforms")),
        "{err}"
    );
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
    let reg = cmd.registry();
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
    let reg = cmd.registry();
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
    // Non-delegated branch (no CI-delegated target): the coordinator OWNS the
    // Release — it is created, not delegated (coordinator-release-vs-cargo-dist-ownership).
    assert!(
        !tag.github_release_delegated,
        "a non-delegated plan must not delegate the Release"
    );
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
    let reg = cmd.registry();
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
fn a_failed_package_preflight_stops_at_dry_run_before_publish_or_tag() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let reg = FakeRegistry::empty();
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    };
    // The cargo build/package step is now `cargo package --no-verify`, and the
    // dry-run phase runs that SAME preflight (a faithful preflight — see
    // `release-cut-build-phase-dep-ordering`), so a package failure surfaces at
    // dry-run-all, before any external effect. Fail `cargo package` and assert the
    // cut stops at the dry-run barrier — publish never runs and no tag is created.
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
            phase: Phase::DryRun,
            ..
        }
    ));
    assert!(
        !cmd.calls().iter().any(|c| c.contains("publish")),
        "published after a failed package preflight"
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
    let reg = cmd.registry();
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
    let reg = FakeRegistry::empty();
    let root = PathBuf::from("/repo");
    let tagger = FakeTagger::new();

    // First attempt: npm publish fails after rust has published.
    {
        let cmd = FakeCmd::failing_on("npm publish").sharing(&reg);
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
    let cmd = FakeCmd::new().sharing(&reg);
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
        !cmd.calls()
            .iter()
            .any(|c| c == "cargo publish --registry crates-io -p tool"),
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
    let reg = cmd.registry();
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
    let cmd = FakeCmd::with_origin("git@github.com:o/r.git").crate_version("1.0.0");
    let reg = cmd.registry();
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
    let reg = cmd.registry();
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
    let reg = cmd.registry();
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
    // The `origin` slug is resolved from the REAL repo root, not the throwaway
    // checkout cwd (`release-cut-clean-checkout` llm-review): reading git config from
    // a `$TMPDIR` worktree risks a silent slug-downgrade under conditional-include /
    // `safe.directory` configs.
    assert!(
        cmd.calls_with_cwd()
            .iter()
            .any(|(line, cwd)| line == "git remote get-url origin" && cwd == &root),
        "the origin slug lookup did not run against the real repo root: {:?}",
        cmd.calls_with_cwd()
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
    let reg = FakeRegistry::empty();
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    };

    // First attempt: the binary publish fails after build completed, leaving the
    // build phase journalled Ok and the run resumable.
    {
        let cmd = FakeCmd::failing_on("gh release upload").sharing(&reg);
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
    let cmd = FakeCmd::new().sharing(&reg);
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
    /// A single command line (exactly rendered `program args`) to fail with a
    /// non-zero exit — for simulating a cut that dies partway through publish-all.
    /// Exact-match (not substring) so failing `… -p ossctl` cannot also catch
    /// `… -p ossctl-core`. A failed publish does **not** mark its crate published.
    fail_line: Option<String>,
}
impl WorkspaceCmd {
    fn new(world: CratesWorld) -> Self {
        Self {
            world,
            calls: RefCell::new(Vec::new()),
            fail_line: None,
        }
    }
    /// A runner that fails exactly the command line `line` (used to interrupt a cut
    /// between the dependency's publish and the dependent's).
    fn failing_line(world: CratesWorld, line: &str) -> Self {
        Self {
            fail_line: Some(line.to_string()),
            ..Self::new(world)
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
        if program == "gh" && args.starts_with(&["release", "view"]) && args.contains(&"--json") {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: r#"{"assets":[{"name":"ossctl-aarch64-apple-darwin.tar.xz"}]}"#.into(),
                stderr: String::new(),
            });
        }
        // Serve the `origin` remote so a homebrew target's cut resolves a slug.
        if program == "git" && args == ["remote", "get-url", "origin"] {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: "git@github.com:jarimustonen/ossctl.git".to_string(),
                stderr: String::new(),
            });
        }
        if program == "curl"
            && args
                .last()
                .is_some_and(|url| url.contains("raw.githubusercontent.com"))
        {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: "# Generated by ossctl; do not edit by hand (template-version: 2)\nclass Ossctl < Formula\n  version \"1.2.3\"\n  if OS.mac? && Hardware::CPU.arm?\n    url \"https://example/ossctl-aarch64-apple-darwin.tar.xz\"\n    sha256 \"deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef\"\n  end\nend\n".into(),
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
        // The homebrew tap-write path reads the tap's current formula from the clone
        // and byte-compares. Fake the clone by checking out an OLDER *ossctl-marked*
        // formula (ossctl's own tap formula is ossctl-generated) so the rendered one
        // differs and the ownership marker authorises a full regenerate → the path
        // overwrites, commits, and pushes.
        if program == "gh" && args.first() == Some(&"repo") && args.get(1) == Some(&"clone") {
            if let Some(workdir) = args.get(3) {
                let dir = Path::new(workdir).join("Formula");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    dir.join("ossctl.rb"),
                    "# Generated by ossctl; do not edit by hand (template-version: 1)\n\
                     class Ossctl < Formula\n  # old\nend\n",
                )
                .unwrap();
            }
            return Ok(CommandOutput {
                status: Some(0),
                stdout: String::new(),
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
        let fails = self.fail_line.as_deref() == Some(line.as_str());
        // A real publish (`cargo publish -p X`, no `--dry-run`) lands the crate — but
        // only if it SUCCEEDS. A failed publish must not mark its crate published (the
        // resume-mid-interleave case: the dependent's publish dies, its crate absent).
        if !fails
            && program == "cargo"
            && args.first() == Some(&"publish")
            && !args.contains(&"--dry-run")
        {
            if let Some(pos) = args.iter().position(|a| *a == "-p") {
                if let Some(pkg) = args.get(pos + 1) {
                    self.world.published.borrow_mut().insert((*pkg).to_string());
                }
            }
        }
        Ok(CommandOutput {
            status: Some(i32::from(fails)),
            stdout: String::new(),
            stderr: if fails { "boom".into() } else { String::new() },
        })
    }
}

/// Registry view over the shared [`CratesWorld`]: reports `1.2.3` once a crate has
/// been published.
struct WorldRegistry {
    world: CratesWorld,
}
impl RegistryQuery for WorldRegistry {
    fn http_get(&self, _url: &str) -> io::Result<(u16, Vec<u8>)> {
        Ok((
            200,
            format!(
                "# Generated by ossctl; do not edit by hand (template-version: 2)\n\
                 class Ossctl < Formula\n  version \"1.2.3\"\n  if OS.mac? && Hardware::CPU.arm?\n    url \"https://example/ossctl-aarch64-apple-darwin.tar.xz\"\n    sha256 \"{CANNED_SHA256}\"\n  end\nend\n"
            )
            .into_bytes(),
        ))
    }

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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
        .position(|c| c == "cargo publish --registry crates-io -p ossctl-core")
        .expect("ossctl-core was published");
    let cli_pub = calls
        .iter()
        .position(|c| c == "cargo publish --registry crates-io -p ossctl")
        .expect("ossctl was published");
    assert!(
        core_pub < cli_pub,
        "dependency published after its dependent: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "cargo publish --registry crates-io -p ossctl-core")
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
            .filter(|c| *c == "cargo publish --registry crates-io -p ossctl-core")
            .count(),
        1,
        "the shared dependency was published more than once under lag: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "cargo publish --registry crates-io -p ossctl")
            .count(),
        1
    );
    // Dependency before dependent.
    let core = calls
        .iter()
        .position(|c| c == "cargo publish --registry crates-io -p ossctl-core")
        .unwrap();
    let cli = calls
        .iter()
        .position(|c| c == "cargo publish --registry crates-io -p ossctl")
        .unwrap();
    assert!(
        core < cli,
        "dependency published after its dependent: {calls:?}"
    );
}

// ── cargo-interleave: dependent packaging defers into publish (ADR-0002) ─────

#[test]
fn dependent_packaging_interleaves_into_publish_not_build_all() {
    // Done criterion (a) — the cargo-ecosystem interleave
    // (`release-cut-build-phase-dep-ordering`, ADR-0002 amendment). Two crates.io
    // targets: the leaf `ossctl-core`, then the dependent `ossctl` (which pins
    // `ossctl-core = "=1.2.3"`). The dependent CANNOT be `cargo package`d in build-all
    // — packaging resolves the `=`-pinned dep against the crates.io index, and that
    // version is only published later. So its packaging is DEFERRED to `cargo publish`
    // in the dep-ordered publish phase, which runs after `ossctl-core` is published
    // and index-visible. The leaf, having no unpublished workspace dep, still packages
    // in build-all. Both land; the run completes.
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
        plan_id: "plan-interleave".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.2.3".into(),
        targets: vec![crates_target("ossctl-core"), crates_target("ossctl")],
        phases: PlanPhase::SEQUENCE.to_vec(),
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    };
    let ids = crate::release::journal_target_ids(&plan.targets);
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-interleave".into(),
        "1.2.3".into(),
        ids,
    )
    .unwrap();
    let mut sink = NullSink;
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    assert_eq!(journal.state().status, RunStatus::Completed);
    let calls = cmd.calls();

    // The DEPENDENT is never packaged as its own build step — it cannot be, before
    // its pinned dep is published. Its packaging happens inside `cargo publish`.
    assert!(
        !calls
            .iter()
            .any(|c| c == "cargo package --registry crates-io -p ossctl --no-verify"),
        "the dependent packaged in build-all instead of deferring: {calls:?}"
    );
    // The LEAF is packaged in build-all (it has no unpublished workspace dep).
    assert!(
        calls
            .iter()
            .any(|c| c == "cargo package --registry crates-io -p ossctl-core --no-verify"),
        "the leaf was not packaged in build-all: {calls:?}"
    );
    // The interleave order: `ossctl-core` publishes, then `ossctl` publishes (its
    // `cargo publish` is where the dependent finally packages, now that its dep is on
    // the index).
    let core_pub = calls
        .iter()
        .position(|c| c == "cargo publish --registry crates-io -p ossctl-core")
        .expect("ossctl-core was published");
    let cli_pub = calls
        .iter()
        .position(|c| c == "cargo publish --registry crates-io -p ossctl")
        .expect("ossctl was published");
    assert!(
        core_pub < cli_pub,
        "the dependent published before its dependency: {calls:?}"
    );
    // The dependent's compile safety net (`cargo check -p ossctl`) still ran BEFORE
    // any publish — the pre-publish barrier over every target is preserved even though
    // its packaging interleaves into publish.
    let cli_check = calls
        .iter()
        .position(|c| c == "cargo check -p ossctl")
        .expect("the dependent's compile gate ran");
    assert!(
        cli_check < core_pub,
        "the dependent's compile gate ran after a publish: {calls:?}"
    );
}

#[test]
fn resume_after_core_publish_completes_the_dependent_without_republishing_core() {
    // Done criterion (b): a cut that dies AFTER publishing the dependency
    // `ossctl-core` but BEFORE publishing the dependent `ossctl` must resume —
    // skipping the already-landed, irreversible `ossctl-core` and completing `ossctl`.
    // The journal (per-target `published` set) and the crates world are shared across
    // the two runs; the second run must never re-run `cargo publish -p ossctl-core`.
    let world = CratesWorld::default();
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");

    let plan = ReleasePlan {
        plan_id: "plan-resume-interleave".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.2.3".into(),
        targets: vec![crates_target("ossctl-core"), crates_target("ossctl")],
        phases: PlanPhase::SEQUENCE.to_vec(),
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    };
    let ids = crate::release::journal_target_ids(&plan.targets);
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-resume-interleave".into(),
        "1.2.3".into(),
        ids,
    )
    .unwrap();
    let mut sink = NullSink;

    // First run: the dependent's publish dies. `ossctl-core` lands (published +
    // journalled); the publish phase fails before `ossctl`.
    let reg = WorldRegistry {
        world: world.clone(),
    };
    let failing = WorkspaceCmd::failing_line(
        world.clone(),
        "cargo publish --registry crates-io -p ossctl",
    );
    let first_ctx = EffectCtx {
        runner: &failing,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let err = execute(&mut journal, &plan, &first_ctx, &tagger, &mut sink).unwrap_err();
    assert!(
        matches!(
            err,
            CutError::PhaseFailed {
                phase: Phase::Publish,
                ..
            }
        ),
        "expected a publish-phase failure, got {err:?}"
    );
    assert!(
        journal.state().published.contains_key("rust:ossctl-core"),
        "the dependency did not land before the interruption"
    );
    assert!(
        !journal.state().published.contains_key("rust:ossctl"),
        "the dependent must not be recorded as published after a failed publish"
    );
    assert!(world.published.borrow().contains("ossctl-core"));
    assert!(!world.published.borrow().contains("ossctl"));

    // Second run: a healthy runner over the SAME journal + crates world. Resume must
    // skip `ossctl-core` (already published) and land `ossctl`.
    let resume_cmd = WorkspaceCmd::new(world.clone());
    let resume_ctx = EffectCtx {
        runner: &resume_cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    execute(&mut journal, &plan, &resume_ctx, &tagger, &mut sink).unwrap();

    assert_eq!(journal.state().status, RunStatus::Completed);
    assert!(journal.state().published.contains_key("rust:ossctl"));
    assert_eq!(journal.state().published.len(), 2);

    let resume_calls = resume_cmd.calls();
    // The already-landed dependency is NEVER re-published on resume (a second
    // `cargo publish -p ossctl-core` would hard-fail on crates.io and is irreversible).
    assert!(
        !resume_calls
            .iter()
            .any(|c| c == "cargo publish --registry crates-io -p ossctl-core"),
        "resume re-published the already-landed dependency: {resume_calls:?}"
    );
    // The dependent's publish completes on resume — now that `ossctl-core` is on the
    // index, its deferred packaging succeeds inside `cargo publish`.
    assert!(
        resume_calls
            .iter()
            .any(|c| c == "cargo publish --registry crates-io -p ossctl"),
        "resume did not complete the dependent's publish: {resume_calls:?}"
    );
}

// ── Plan validation (before any external action) ─────────────────────────────

#[test]
fn refuses_a_target_with_no_resolved_package() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new();
    let reg = cmd.registry();
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
    let cmd = FakeCmd::new().crate_version("1.0.0");
    let reg = cmd.registry();
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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

#[test]
fn ci_delegated_homebrew_skips_dist_write_but_verifies_unmarked_formula() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new();
    let registry = UnmarkedFormulaRegistry;
    let tagger = FakeTagger::new();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let plan = ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 2,
        head_sha: "d".into(),
        version: "1.0.0".into(),
        targets: vec![PlanTarget {
            ecosystem: Ecosystem::Rust,
            package: Some("tool".into()),
            registry: Registry::Homebrew,
            adapter: Adapter::CargoDist,
        }],
        phases: PlanPhase::SEQUENCE.to_vec(),
        bump: None,
        homebrew_tap: Some("owner/homebrew-tool".into()),
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
    let mut sink = RecordingSink::default();

    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    assert_eq!(journal.state().status, RunStatus::Completed);
    assert!(journal.state().delegated.contains(&ids[0]));
    assert!(!journal.state().published.contains_key(&ids[0]));
    assert_eq!(
        journal.state().verified.get(&ids[0]),
        Some(&VerifyOutcome::Matches)
    );
    let calls = cmd.calls();
    assert!(
        !calls.iter().any(|call| {
            call.contains("repo clone")
                || call.contains("push origin")
                || call.contains("formula update")
        }),
        "CI-delegated Homebrew target must never write the tap: {calls:?}"
    );
}

#[test]
fn ci_delegated_homebrew_requires_every_planned_platform_stanza() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(Cell::new(1000));
    let registry = UnmarkedFormulaRegistry;
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut plan = delegated_plan();
    plan.version = "1.0.0".into();
    plan.homebrew_tap = Some("owner/homebrew-tool".into());
    plan.homebrew_platforms = vec![
        "aarch64-apple-darwin".into(),
        "x86_64-unknown-linux-musl".into(),
    ];

    assert_eq!(
        verify_delegated_homebrew(&ctx, &plan, "tool"),
        VerifyOutcome::Missing
    );
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
        bump: None,
        homebrew_tap: Some("jarimustonen/homebrew-ossctl".into()),
        license: Some("MIT".into()),
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
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
            .filter(|c| *c == "cargo publish --registry crates-io -p ossctl-core")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "cargo publish --registry crates-io -p ossctl")
            .count(),
        1
    );
    let core = calls
        .iter()
        .position(|c| c == "cargo publish --registry crates-io -p ossctl-core")
        .unwrap();
    let cli = calls
        .iter()
        .position(|c| c == "cargo publish --registry crates-io -p ossctl")
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
    // The homebrew leg is self-sufficient: no `brew bump-formula-pr` (and thus no
    // `brew audit`), it renders the formula and pushes it straight to the tap's
    // default branch. The push is what makes `brew install` resolve the new version.
    assert!(
        !calls.iter().any(|c| c.contains("bump-formula-pr")),
        "homebrew must not shell to bump-formula-pr for a configured tap: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("gh repo clone jarimustonen/homebrew-ossctl ")),
        "homebrew leg did not clone the tap: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.ends_with("push origin HEAD")),
        "homebrew leg did not push the formula to the tap: {calls:?}"
    );
    // The formula it wrote to the tap checkout carries the real post-tag sha256 the
    // dist phase computed by hashing the fetched tag archive.
    let hb_prefix = "ossctl-homebrew-ossctl-1.2.3-";
    let hb_dir = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(hb_prefix))
        })
        .expect("the tap-write path made a scratch checkout");
    let formula = std::fs::read_to_string(hb_dir.join("Formula/ossctl.rb"))
        .expect("the tap-write path wrote the formula file");
    assert!(
        formula.contains(&format!("sha256 \"{CANNED_SHA256}\"")),
        "formula lacks the real post-tag sha256: {formula}"
    );
    assert!(
        formula.contains(
            "url \"https://github.com/jarimustonen/ossctl/releases/download/v1.2.3/ossctl-aarch64-apple-darwin.tar.xz\""
        ),
        "formula lacks the prebuilt release-asset url: {formula}"
    );
    let _ = std::fs::remove_dir_all(&hb_dir);

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

// ── GitHub Release ownership vs CI (coordinator-release-vs-cargo-dist-ownership) ─

/// A plan with a crates.io target + a cargo-dist (CI-delegated) target.
fn delegated_plan() -> ReleasePlan {
    ReleasePlan {
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    }
}

#[test]
fn a_ci_delegated_plan_delegates_the_github_release_and_never_creates_it() {
    // Option 1 (coordinator-release-vs-cargo-dist-ownership): when the plan carries
    // a CI-delegated target (cargo-dist), the coordinator creates AND pushes the tag
    // — that pushed tag is what triggers cargo-dist's `release.yml` — but does NOT
    // create the GitHub Release: CI owns Release creation + the cross-platform binary
    // upload. The delegation is journalled so resume/verify don't treat the missing
    // engine-created Release as a step to re-attempt.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new().crate_version("1.0.0");
    let reg = cmd.registry();
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

    let plan = delegated_plan();
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
    let tag = state.tags.get("v1.0.0").expect("tag recorded");
    // Tag created + pushed, but the Release was DELEGATED, not created.
    assert!(
        tag.created_local && tag.pushed_remote,
        "the tag itself must still be created and pushed"
    );
    assert!(
        tag.github_release_delegated,
        "the Release delegation was not journalled"
    );
    assert!(
        !tag.github_release,
        "the coordinator created a Release for a CI-delegated plan (double-create clash)"
    );
    // The Tagger's create_github_release was NEVER called; create + push still were.
    assert!(
        !tagger.calls().iter().any(|c| c.starts_with("release:")),
        "coordinator created a GitHub Release despite CI delegation: {:?}",
        tagger.calls()
    );
    assert_eq!(tagger.calls(), vec!["create:v1.0.0@d", "push:v1.0.0"]);
    // Exactly one delegation event was streamed (and no created event), after the push,
    // and it names the owning adapter (cargo-dist) for the operator record.
    let k = &sink.kinds;
    assert_eq!(
        k.iter()
            .filter(
                |e| matches!(e, EventKind::GithubReleaseDelegated { delegated_to, .. } if delegated_to == "cargo-dist")
            )
            .count(),
        1,
        "expected exactly one github_release_delegated event naming cargo-dist"
    );
    assert!(
        !k.iter()
            .any(|e| matches!(e, EventKind::GithubReleaseCreated { .. })),
        "a github_release_created event was streamed for a delegated plan"
    );
    let deleg = first_idx(k, |e| matches!(e, EventKind::GithubReleaseDelegated { .. })).unwrap();
    let push = first_idx(k, |e| matches!(e, EventKind::TagPushedRemote { .. })).unwrap();
    assert!(push < deleg, "delegation must follow the tag push");
}

#[test]
fn a_resumed_delegated_run_completes_without_ever_creating_the_release() {
    // A cut interrupted mid-tag (the push failed) must, on resume, retry the push,
    // record the delegation, and complete — and across BOTH attempts the coordinator
    // must never create the GitHub Release. This is the resume-safety the issue
    // requires: a resumed CI-delegated run does not re-attempt (nor first-attempt)
    // the engine-owned Release.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let reg = FakeRegistry::empty();
    let root = PathBuf::from("/repo");
    let plan = delegated_plan();
    let ids = crate::release::journal_target_ids(&plan.targets);

    // First attempt: the tag push fails, after the local tag is created but before
    // any Release step.
    let first_tagger = FakeTagger::failing("push");
    {
        let cmd = FakeCmd::new().crate_version("1.0.0").sharing(&reg);
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
            "p".into(),
            "1.0.0".into(),
            ids.clone(),
        )
        .unwrap();
        execute(&mut journal, &plan, &ctx, &first_tagger, &mut sink).unwrap_err();
        let tag = journal
            .state()
            .tags
            .get("v1.0.0")
            .expect("local tag recorded");
        assert!(tag.created_local && !tag.pushed_remote);
        assert!(!tag.github_release && !tag.github_release_delegated);
    } // journal drops → releases the lock

    // Resume: the push succeeds now, the delegation is recorded, the run completes.
    let resume_tagger = FakeTagger::new();
    let cmd = FakeCmd::new().crate_version("1.0.0").sharing(&reg);
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let mut sink = RecordingSink::default();
    let mut journal = Journal::open(&store, &clock, paths(), "RUN01").unwrap();
    execute(&mut journal, &plan, &ctx, &resume_tagger, &mut sink).unwrap();

    let state = journal.state();
    assert_eq!(state.status, RunStatus::Completed);
    let tag = state.tags.get("v1.0.0").unwrap();
    assert!(tag.created_local && tag.pushed_remote);
    assert!(
        tag.github_release_delegated && !tag.github_release,
        "resume must delegate the Release, never create it"
    );
    // The already-created local tag was NOT re-created; only the push was retried.
    assert_eq!(resume_tagger.calls(), vec!["push:v1.0.0"]);
    // Neither attempt's tagger ever created a Release.
    assert!(
        !first_tagger
            .calls()
            .iter()
            .chain(resume_tagger.calls().iter())
            .any(|c| c.starts_with("release:")),
        "a GitHub Release was created across the resumed CI-delegated run: {:?} / {:?}",
        first_tagger.calls(),
        resume_tagger.calls()
    );
    // The delegation was recorded exactly once (on the resume that completed the tag).
    assert_eq!(
        sink.kinds
            .iter()
            .filter(|e| matches!(e, EventKind::GithubReleaseDelegated { .. }))
            .count(),
        1
    );
}

/// Build a journal whose tag is already through `created_local` + `pushed_remote`,
/// plus whichever Release-disposition event `disposition` appends — the setup for
/// the contradictory-disposition guard tests.
fn journal_with_tag_disposition<'a>(
    store: &'a FakeStore,
    clock: &'a FakeClock,
    idgen: &'a FakeIdGen,
    disposition: EventKind,
) -> Journal<'a> {
    let mut journal = Journal::create(
        store,
        clock,
        idgen,
        paths(),
        "p".into(),
        "1.0.0".into(),
        vec!["rust".into()],
    )
    .unwrap();
    for ev in [
        EventKind::TagCreatedLocal {
            tag: "v1.0.0".into(),
        },
        EventKind::TagPushedRemote {
            tag: "v1.0.0".into(),
        },
        disposition,
    ] {
        journal.append(ev).unwrap();
    }
    journal
}

#[test]
fn delegating_over_an_already_created_release_is_refused_not_double_recorded() {
    // Contradictory disposition (unreachable for a fixed plan_id, but possible if a
    // resumed run's binary reclassifies the adapter): the journal already has an
    // engine-created Release, yet the plan now delegates it to CI. The tag phase must
    // FAIL — never append a delegation on top of a creation (dual-disposition state).
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let tagger = FakeTagger::new();
    let mut sink = RecordingSink::default();
    let mut journal = journal_with_tag_disposition(
        &store,
        &clock,
        &idgen,
        EventKind::GithubReleaseCreated {
            tag: "v1.0.0".into(),
            url: Some("https://github.com/x/y/releases/v1.0.0".into()),
        },
    );

    let plan = delegated_plan();
    let err = tag_phase(
        &mut journal,
        &mut sink,
        &tagger,
        &plan,
        &plan.head_sha,
        Some("cargo-dist"),
    )
    .unwrap_err();
    match err {
        CutError::PhaseFailed { phase, target, .. } => {
            assert_eq!(phase, Phase::Tag);
            assert_eq!(target, None);
        }
        other => panic!("expected a tag PhaseFailed, got {other:?}"),
    }
    // No delegation event was recorded, and the Release flag stays created-only.
    let tag = journal.state().tags.get("v1.0.0").unwrap();
    assert!(tag.github_release && !tag.github_release_delegated);
    assert!(!sink
        .kinds
        .iter()
        .any(|e| matches!(e, EventKind::GithubReleaseDelegated { .. })));
}

#[test]
fn creating_over_an_already_delegated_release_is_refused() {
    // The reverse contradiction: the journal already delegated the Release to CI, yet
    // the plan now has the coordinator create it. Creating it would clash with the CI
    // that may already have created it, so the tag phase must FAIL rather than call
    // create_github_release.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let tagger = FakeTagger::new();
    let mut sink = RecordingSink::default();
    let mut journal = journal_with_tag_disposition(
        &store,
        &clock,
        &idgen,
        EventKind::GithubReleaseDelegated {
            tag: "v1.0.0".into(),
            delegated_to: "cargo-dist".into(),
        },
    );

    // A non-delegated (coordinator-owned) plan at the same version as the journal tag
    // (v1.0.0): release_owner = None.
    let plan = ReleasePlan {
        version: "1.0.0".into(),
        ..two_target_plan()
    };
    let err = tag_phase(
        &mut journal,
        &mut sink,
        &tagger,
        &plan,
        &plan.head_sha,
        None,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        CutError::PhaseFailed {
            phase: Phase::Tag,
            ..
        }
    ));
    // create_github_release was never called; the flag stays delegated-only.
    assert!(!tagger.calls().iter().any(|c| c.starts_with("release:")));
    let tag = journal.state().tags.get("v1.0.0").unwrap();
    assert!(tag.github_release_delegated && !tag.github_release);
}

// ── Integration: a cut actually UPLOADS to a (mock) registry ─────────────────
//
// The `release-cut-publish-noop` regression: a real cut reported `cargo` success
// yet nothing reached crates.io. These tests wire the publish through a shared
// mock registry — the publishing runner "uploads" by making the crate visible on
// the same registry the RegistryQuery port reads — and assert on REGISTRY STATE,
// not merely a green cut, so a publish that runs `cargo publish` but uploads
// nothing is caught.

/// A shared in-memory registry the publishing runner writes and the
/// [`RegistryQuery`] port reads — the mock crates.io index the test asserts on.
#[derive(Clone, Default)]
struct SharedRegistry {
    inner: Rc<RefCell<HashMap<String, Vec<String>>>>,
}
impl SharedRegistry {
    fn versions(&self, package: &str) -> Vec<String> {
        self.inner
            .borrow()
            .get(package)
            .cloned()
            .unwrap_or_default()
    }
}
impl RegistryQuery for SharedRegistry {
    fn published_versions(&self, _ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        Ok(self.versions(package))
    }
}

/// A command runner that models a real `cargo publish`: on `cargo publish … -p X`
/// it records X at **the version declared for X in the served `cargo metadata`**
/// — i.e. the manifest version cargo would actually upload — into the shared
/// registry, so the adapter's `is_published` / `wait_for_index` probes (reading the
/// same registry) observe the landed version. Publishing the *metadata* version
/// (not a value handed in out-of-band) keeps the mock faithful to the real
/// "`cargo publish` uploads the tree manifest version" behavior: a coordinator that
/// somehow published a different version than the manifest declares would show up as
/// the wrong version on the registry. `noop = true` is the defect variant: it runs
/// the publish command but uploads nothing.
struct PublishingCmd {
    reg: SharedRegistry,
    metadata: String,
    noop: bool,
    calls: RefCell<Vec<String>>,
}
impl PublishingCmd {
    fn new(reg: SharedRegistry, metadata: &str, noop: bool) -> Self {
        Self {
            reg,
            metadata: metadata.to_string(),
            noop,
            calls: RefCell::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
    /// The version the served metadata declares for `package` (what `cargo publish`
    /// would upload), or `None` if the crate is not in the graph.
    fn manifest_version(&self, package: &str) -> Option<String> {
        let meta: serde_json::Value = serde_json::from_str(&self.metadata).ok()?;
        meta.get("packages")?.as_array()?.iter().find_map(|p| {
            (p.get("name")?.as_str()? == package)
                .then(|| p.get("version")?.as_str().map(str::to_string))
                .flatten()
        })
    }
}
impl CommandRunner for PublishingCmd {
    fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
        self.calls
            .borrow_mut()
            .push(format!("{program} {}", args.join(" ")));
        if program == "cargo" && args.contains(&"metadata") {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: self.metadata.clone(),
                stderr: String::new(),
            });
        }
        if program == "cargo" && args.first() == Some(&"publish") {
            if !self.noop {
                // The `-p <pkg>` that names the one crate this publish uploads, at
                // the version its manifest (the served metadata) declares.
                if let Some(pos) = args.iter().position(|a| *a == "-p") {
                    if let Some(pkg) = args.get(pos + 1) {
                        if let Some(version) = self.manifest_version(pkg) {
                            self.reg
                                .inner
                                .borrow_mut()
                                .entry((*pkg).to_string())
                                .or_default()
                                .push(version);
                        }
                    }
                }
            }
            return Ok(CommandOutput {
                status: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        Ok(CommandOutput {
            status: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

/// `acme-core` (leaf) + `acme` (depends on core), both at 1.2.3 — the multi-crate
/// shape that broke in the issuectl repro (a dependent that index-waits on its
/// workspace dependency).
const TWO_CRATE_METADATA: &str = r#"{"packages":[
  {"name":"acme-core","version":"1.2.3","id":"acme-core 1.2.3","dependencies":[],"publish":null},
  {"name":"acme","version":"1.2.3","id":"acme 1.2.3","dependencies":[{"name":"acme-core","kind":null}],"publish":null}
],"workspace_members":["acme-core 1.2.3","acme 1.2.3"]}"#;

const ONE_CRATE_METADATA: &str = r#"{"packages":[
  {"name":"acme-core","version":"1.2.3","id":"acme-core 1.2.3","dependencies":[],"publish":null}
],"workspace_members":["acme-core 1.2.3"]}"#;

fn rust_crate_plan(packages: &[&str]) -> ReleasePlan {
    ReleasePlan {
        plan_id: "plan-test".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.2.3".into(),
        targets: packages
            .iter()
            .map(|p| PlanTarget {
                ecosystem: Ecosystem::Rust,
                package: Some((*p).to_string()),
                registry: Registry::CratesIo,
                adapter: Adapter::CargoPublish,
            })
            .collect(),
        phases: PlanPhase::SEQUENCE.to_vec(),
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    }
}

#[test]
fn cut_actually_publishes_both_crates_to_the_mock_registry() {
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let reg = SharedRegistry::default();
    let cmd = PublishingCmd::new(reg.clone(), TWO_CRATE_METADATA, false);
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
    let plan = rust_crate_plan(&["acme-core", "acme"]);
    let ids = crate::release::journal_target_ids(&plan.targets);
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-test".into(),
        "1.2.3".into(),
        ids.clone(),
    )
    .unwrap();

    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();

    // The crates ACTUALLY appear on the (mock) registry at the cut version — the
    // check `cargo exited 0` alone would miss (`release-cut-publish-noop`).
    assert_eq!(reg.versions("acme-core"), vec!["1.2.3".to_string()]);
    assert_eq!(reg.versions("acme"), vec!["1.2.3".to_string()]);
    // A real `cargo publish` ran for each crate (not skipped as already-published).
    let calls = cmd.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("publish") && c.contains("-p acme-core")),
        "expected a cargo publish of acme-core; calls: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("publish") && c.contains("-p acme")),
        "expected a cargo publish of acme; calls: {calls:?}"
    );
    // And each landed target has a per-member receipt at the cut version.
    let state = journal.state();
    assert_eq!(state.published.len(), 2, "one receipt per crate");
    for id in &ids {
        assert_eq!(
            state.published.get(id).expect("receipt present").version,
            "1.2.3",
            "receipt for {id}"
        );
    }
}

#[test]
fn a_noop_publish_fails_the_cut_with_no_fabricated_receipt() {
    // A single leaf crate + a runner that runs `cargo publish` but uploads NOTHING —
    // the `release-cut-publish-noop` / issuectl 0.8.1 signature. cargo "succeeds", but
    // the self-visibility confirm (`cut-noop-self-visibility-check`) probes the registry
    // for the crate's OWN {name, version} after the publish and never finds it, so the
    // cut FAILS LOUDLY at publish-all and journals NO receipt — the fabricated-success
    // is turned into a hard failure. This is the fix's integration proof: assert the cut
    // fails, the journal has no receipt, and the registry stayed empty.
    let store = FakeStore::default();
    // A virtual-time clock: the self-visibility confirm polls the (empty) registry
    // and sleeps between polls, so a real-sleeping clock would take the full 300s
    // ceiling. This advances virtual time on `sleep`, terminating the bounded wait
    // instantly and deterministically.
    let clock = SleepAdvancingClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let reg = SharedRegistry::default();
    let cmd = PublishingCmd::new(reg.clone(), ONE_CRATE_METADATA, true);
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
    let plan = rust_crate_plan(&["acme-core"]);
    let ids = crate::release::journal_target_ids(&plan.targets);
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-test".into(),
        "1.2.3".into(),
        ids,
    )
    .unwrap();

    // The cut FAILS at publish-all — the confirm caught the no-op.
    let err = execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap_err();
    match err {
        CutError::PhaseFailed { phase, .. } => assert_eq!(phase, Phase::Publish),
        other => panic!("expected a publish PhaseFailed on the no-op, got {other:?}"),
    }
    // NO receipt was fabricated for the crate that never landed, and the run neither
    // completed nor tagged.
    assert!(
        journal.state().published.is_empty(),
        "a no-op publish must not journal a fabricated receipt"
    );
    assert_ne!(journal.state().status, RunStatus::Completed);
    assert!(tagger.calls().is_empty(), "a failed publish must not tag");
    // The registry genuinely stayed empty — the confirm's premise held.
    assert!(reg.versions("acme-core").is_empty());
}

#[test]
fn a_real_publish_journals_a_receipt_and_lands_on_the_registry() {
    // The passing counterpart to the no-op test: a runner that ACTUALLY uploads makes
    // the crate visible on the shared mock registry, so the self-visibility confirm
    // passes, the receipt is journaled, and the cut completes + tags. Guards against
    // the confirm making a normal cut flaky.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let reg = SharedRegistry::default();
    let cmd = PublishingCmd::new(reg.clone(), ONE_CRATE_METADATA, false);
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
    let plan = rust_crate_plan(&["acme-core"]);
    let ids = crate::release::journal_target_ids(&plan.targets);
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths(),
        "plan-test".into(),
        "1.2.3".into(),
        ids.clone(),
    )
    .unwrap();

    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).unwrap();
    // The confirm saw the crate land, so a receipt is journaled at the cut version …
    let state = journal.state();
    assert_eq!(state.published.len(), 1);
    assert_eq!(
        state
            .published
            .get(&ids[0])
            .expect("receipt present")
            .version,
        "1.2.3"
    );
    // … and the crate is genuinely on the (mock) registry.
    assert_eq!(reg.versions("acme-core"), vec!["1.2.3".to_string()]);
}

// ── Clean checkout of the sealed commit (release-cut-clean-checkout) ──────────

/// The path `git worktree add --detach <path> <sha>` created — the sealed-commit
/// checkout every effect command must run from. Panics if the checkout was never
/// materialized (a test that expected it to be).
fn sealed_checkout_path(cmd: &FakeCmd) -> PathBuf {
    for (line, _) in cmd.calls_with_cwd() {
        // `git worktree add --detach <path> <sha>`
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.first() == Some(&"git")
            && toks.get(1) == Some(&"worktree")
            && toks.get(2) == Some(&"add")
            && toks.get(3) == Some(&"--detach")
        {
            return PathBuf::from(toks[4]);
        }
    }
    panic!(
        "no `git worktree add --detach` call was recorded: {:?}",
        cmd.calls()
    );
}

#[test]
fn effect_commands_run_from_the_sealed_commit_checkout() {
    // A cut materializes a clean checkout of `plan.head_sha` and runs every effect
    // command (build/publish/dist) from THERE, not the live repo root — so a mid-cut
    // edit of the operator's tree can never change what is published.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new();
    let reg = cmd.registry();
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

    // `two_target_plan` seals `head_sha: "deadbeef"`.
    execute(&mut journal, &two_target_plan(), &ctx, &tagger, &mut sink).unwrap();

    let checkout = sealed_checkout_path(&cmd);
    assert_ne!(
        checkout, root,
        "the checkout must not be the live repo root"
    );

    let calls = cmd.calls_with_cwd();
    // The worktree was checked out at the SEALED commit (not whatever HEAD is now).
    assert!(
        calls.iter().any(|(line, cwd)| line
            == &format!("git worktree add --detach {} deadbeef", checkout.display())
            && cwd == &root),
        "worktree add did not check out the sealed commit from the real root: {calls:?}"
    );
    // The worktree admin commands (add / probe / remove) run against the REAL repo;
    // the checkout only hosts the effect commands.
    assert!(
        calls
            .iter()
            .any(|(line, cwd)| line == "git cat-file -e deadbeef^{commit}" && cwd == &root),
        "the sealed-commit presence probe did not run against the real root: {calls:?}"
    );

    // Every cargo/npm effect command ran from the checkout — and NONE from the live
    // repo root (the immunity-to-a-dirty-live-tree property).
    let mut saw_effect = false;
    for (line, cwd) in &calls {
        if line.starts_with("cargo ") || line.starts_with("npm ") {
            saw_effect = true;
            assert_eq!(
                cwd, &checkout,
                "an effect command ran outside the sealed checkout: `{line}` in {cwd:?}"
            );
        }
    }
    assert!(
        saw_effect,
        "no cargo/npm effect command ran at all: {calls:?}"
    );

    // The checkout is torn down afterwards (Drop), against the real repo root.
    assert!(
        calls.iter().any(|(line, cwd)| line
            == &format!("git worktree remove --force {}", checkout.display())
            && cwd == &root),
        "the checkout worktree was not torn down against the real root: {calls:?}"
    );
}

#[test]
fn missing_sealed_commit_fails_closed_before_any_effect() {
    // If the sealed commit is not present locally, a cut refuses BEFORE any phase —
    // no dry-run, no build, no publish, no tag — rather than fall back to the live tree.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::missing_sealed_commit();
    let reg = cmd.registry();
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
    assert!(
        matches!(err, CutError::Checkout(_)),
        "expected a fail-closed CutError::Checkout, got {err:?}"
    );

    let calls = cmd.calls();
    // The presence probe ran; nothing else did.
    assert!(
        calls
            .iter()
            .any(|c| c == "git cat-file -e deadbeef^{commit}"),
        "the sealed-commit probe should have run: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.starts_with("git worktree add")),
        "no worktree should be created when the commit is absent: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|c| c.starts_with("cargo ") || c.starts_with("npm ")),
        "no effect command may run when the sealed commit is absent: {calls:?}"
    );
    assert!(tagger.calls().is_empty(), "a fail-closed cut must not tag");
    // No phase was even entered.
    assert!(
        journal.state().phases.is_empty(),
        "no phase should be recorded on a fail-closed checkout"
    );
}

#[test]
fn checkout_is_torn_down_even_when_a_phase_fails() {
    // A phase failure still tears the checkout worktree down (Drop runs on the error
    // path), so a failed cut never leaks a throwaway worktree.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::failing_on("npm publish");
    let reg = cmd.registry();
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
    assert!(
        matches!(err, CutError::PhaseFailed { .. }),
        "expected a publish-phase failure, got {err:?}"
    );

    let checkout = sealed_checkout_path(&cmd);
    assert!(
        cmd.calls()
            .iter()
            .any(|c| c == &format!("git worktree remove --force {}", checkout.display())),
        "the checkout must be torn down even on a phase failure: {:?}",
        cmd.calls()
    );
}

// ── Engine-owned version bump (release-rust-workspace-multicrate facet 2/3) ───

/// Recursively copy a directory tree (the seed workspace → the sealed checkout dest).
fn copy_tree(src: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

/// A unique throwaway lib+bin workspace at 0.4.0, the bin pinning the lib `=0.4.0`.
/// The fake `git worktree add` copies this seed into the sealed checkout; [`TempDir`]
/// removes it after the test.
fn seed_workspace() -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("ossctl-coord-bump-seed-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("crates/core")).unwrap();
    std::fs::create_dir_all(root.join("crates/cli")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/core\", \"crates/cli\"]\n\n[workspace.package]\nversion = \"0.4.0\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/core/Cargo.toml"),
        "[package]\nname = \"acme-core\"\nversion.workspace = true\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/cli/Cargo.toml"),
        "[package]\nname = \"acme\"\nversion.workspace = true\n\n[dependencies]\nacme-core = { path = \"../core\", version = \"=0.4.0\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("CHANGELOG.md"),
        "# Changelog\n\n## [Unreleased]\n### Added\n- a feature\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Cargo.lock"),
        "# auto\n[[package]]\nname = \"acme-core\"\nversion = \"0.4.0\"\n",
    )
    .unwrap();
    dir
}

/// A single-crates.io-target plan carrying a `--bump` phase (0.4.0 → 0.5.0) with one pin
/// rewrite. The publish version is the BUMPED 0.5.0 (what the tree becomes).
fn bump_plan() -> ReleasePlan {
    ReleasePlan {
        plan_id: "plan-bump".into(),
        contract_schema_version: 1,
        head_sha: "sealedhead".into(),
        version: "0.5.0".into(),
        // The publish target uses the canned-metadata crate name (`tool`); the bump
        // edits (below) concern the seeded acme/acme-core workspace independently.
        targets: vec![PlanTarget {
            ecosystem: Ecosystem::Rust,
            package: Some("tool".into()),
            registry: Registry::CratesIo,
            adapter: Adapter::CargoPublish,
        }],
        phases: {
            let mut p = vec![PlanPhase::Bump];
            p.extend_from_slice(&PlanPhase::SEQUENCE);
            p
        },
        bump: Some(BumpPlan {
            level: BumpLevel::Minor,
            from_version: "0.4.0".into(),
            to_version: "0.5.0".into(),
            pin_rewrites: vec![PinRewrite {
                in_package: "acme".into(),
                dependency: "acme-core".into(),
                from: "=0.4.0".into(),
                to: "=0.5.0".into(),
            }],
            changelog_finalize: true,
            bump_hook: None,
        }),
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    }
}

#[test]
fn a_bump_plan_applies_the_bump_tags_the_bump_commit_and_completes() {
    let seed = seed_workspace();
    let cmd = FakeCmd::new()
        .with_bump_checkout(seed.path().to_path_buf(), "bumpsha00")
        .crate_version("0.5.0");
    let registry = cmd.registry();
    let clock = FakeClock(Cell::new(1_786_579_200)); // 2026-08-13
    let idgen = FakeIdGen("RUNBUMP".into());
    let tagger = FakeTagger::new();
    let store = FakeStore::default();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };

    let plan = bump_plan();
    let mut journal = Journal::create_bump(
        &store,
        &clock,
        &idgen,
        paths(),
        plan.plan_id.clone(),
        plan.version.clone(),
        vec!["rust".into()],
        plan.head_sha.clone(),
        crate::protocol::journal::BumpInputs {
            level: "minor".into(),
            from_version: "0.4.0".into(),
        },
    )
    .unwrap();
    let mut sink = RecordingSink::default();

    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).expect("bump cut completes");

    // The bump was applied + journalled, and the tag points at the BUMP commit.
    let state = journal.state();
    assert_eq!(
        state.bump.as_ref().map(|b| b.commit.as_str()),
        Some("bumpsha00"),
        "BumpApplied recorded the bump commit"
    );
    assert_eq!(
        state.bump.as_ref().map(|b| b.effective_date.as_str()),
        Some("2026-08-13")
    );
    assert!(
        tagger
            .calls()
            .iter()
            .any(|c| c == "create:v0.5.0@bumpsha00"),
        "the tag must point at the bump commit, not the sealed head: {:?}",
        tagger.calls()
    );
    assert_eq!(state.status, RunStatus::Completed);

    // The bump edits landed in the checkout: the seeded copy the fake worktree wrote is
    // torn down, so assert via the runner calls that the bump commit + lockfile refresh ran.
    assert!(cmd.calls().iter().any(|c| c == "cargo update --workspace"));
    assert!(cmd
        .calls()
        .iter()
        .any(|c| c.starts_with("git commit -m release: v0.5.0")));
}

#[test]
fn a_resumed_bump_run_does_not_double_bump() {
    // A run whose journal already carries BumpApplied (phase interrupted before its
    // completion) must NOT re-apply the bump on re-entry.
    let seed = seed_workspace();
    let cmd = FakeCmd::new()
        .with_bump_checkout(seed.path().to_path_buf(), "bumpsha00")
        .crate_version("0.5.0");
    let registry = cmd.registry();
    let clock = FakeClock(Cell::new(1_786_579_200));
    let idgen = FakeIdGen("RUNBUMP2".into());
    let tagger = FakeTagger::new();
    let store = FakeStore::default();
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let plan = bump_plan();
    let mut journal = Journal::create_bump(
        &store,
        &clock,
        &idgen,
        paths(),
        plan.plan_id.clone(),
        plan.version.clone(),
        vec!["rust".into()],
        plan.head_sha.clone(),
        crate::protocol::journal::BumpInputs {
            level: "minor".into(),
            from_version: "0.4.0".into(),
        },
    )
    .unwrap();
    // Pre-seed a BumpApplied fact (as if a prior attempt applied it), then re-enter.
    journal
        .append(EventKind::BumpApplied {
            commit: "priorbump".into(),
            effective_date: "2026-08-01".into(),
        })
        .unwrap();
    let mut sink = RecordingSink::default();
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).expect("resumed bump completes");

    // No second commit was made (the bump was NOT re-applied), and the tag points at the
    // ALREADY-recorded bump commit.
    assert!(
        !cmd.calls().iter().any(|c| c.starts_with("git commit")),
        "a resumed bump must not re-commit: {:?}",
        cmd.calls()
    );
    assert!(tagger
        .calls()
        .iter()
        .any(|c| c == "create:v0.5.0@priorbump"));
    // CRITICAL (llm-review): the clean checkout was materialized AT the recorded bump
    // commit, so dry-run/build/publish operate on the BUMPED tree — never the pre-bump
    // sealed head (which would publish the OLD version under a bumped tag).
    assert!(
        cmd.calls()
            .iter()
            .any(|c| c.starts_with("git worktree add --detach") && c.ends_with("priorbump")),
        "resume must check out the bump commit, not the sealed head: {:?}",
        cmd.calls()
    );
}

// ── CI-delegated crates.io publish: the tag-only cut (release-ci-publish-mode) ─

/// A registry that reports `package@version` only after `appear_after` lookups —
/// the CI-delegated publish landing while the verify barrier polls. Models the real
/// shape: the tag push triggers a workflow that takes minutes to run `cargo publish`
/// and reach the index, so the first observations legitimately see nothing.
struct LaggingRegistry {
    version: String,
    appear_after: Cell<u32>,
}
impl LaggingRegistry {
    fn new(version: &str, appear_after: u32) -> Self {
        Self {
            version: version.to_string(),
            appear_after: Cell::new(appear_after),
        }
    }
}
impl RegistryQuery for LaggingRegistry {
    fn published_versions(&self, _ecosystem: &str, _package: &str) -> io::Result<Vec<String>> {
        let remaining = self.appear_after.get();
        if remaining == 0 {
            return Ok(vec![self.version.clone()]);
        }
        self.appear_after.set(remaining - 1);
        Ok(Vec::new())
    }
}

/// A registry that cannot be reached at all — an outage must never be read as
/// "CI did not publish".
struct UnreachableRegistry;
impl RegistryQuery for UnreachableRegistry {
    fn published_versions(&self, _ecosystem: &str, _package: &str) -> io::Result<Vec<String>> {
        Err(io::Error::other("registry unreachable"))
    }
}

/// A single crates.io target whose publish is CI-delegated (`cargo-publish-ci`) —
/// the glasspad shape: the engine gates and tags, the tag-triggered workflow
/// publishes.
fn ci_publish_plan() -> ReleasePlan {
    ReleasePlan {
        plan_id: "p".into(),
        contract_schema_version: 1,
        head_sha: "d".into(),
        version: "1.0.0".into(),
        targets: vec![plan_target(
            Ecosystem::Rust,
            Registry::CratesIo,
            Adapter::CargoPublishCi,
        )],
        phases: PlanPhase::SEQUENCE.to_vec(),
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec!["aarch64-apple-darwin".into()],
    }
}

#[test]
fn ci_delegated_crates_io_target_tags_without_publishing_and_verify_observes_the_index() {
    // The whole point of the mode: `cargo publish` NEVER runs from this host (the
    // repo's token lives in CI), the tag is still created + pushed by the engine —
    // that push is what triggers the publish workflow — and the run only completes
    // once verify OBSERVES the crate on the registry index.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new().crate_version("1.0.0");
    // The publish lands mid-verify, after a few polls — the engine waits for CI
    // rather than racing it.
    let reg = LaggingRegistry::new("1.0.0", 3);
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

    let plan = ci_publish_plan();
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
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).expect("a tag-only cut completes");

    let state = journal.state();
    assert_eq!(state.status, RunStatus::Completed);
    // Delegated, not published — and never a failure.
    assert!(state.delegated.contains(&ids[0]));
    assert!(!state.published.contains_key(&ids[0]));
    assert!(
        !cmd.calls()
            .iter()
            .any(|c| c.starts_with("cargo publish") && !c.contains("--dry-run")),
        "the engine published locally for a CI-delegated target: {:?}",
        cmd.calls()
    );
    // The local preflight gates STILL ran — that is their whole value before an
    // irreversible tag push.
    assert!(cmd.calls().iter().any(|c| c.starts_with("cargo check")));
    assert!(cmd.calls().iter().any(|c| c.starts_with("cargo package")));
    // The tag is the terminal actionable step, and it was taken.
    assert!(tagger.calls().iter().any(|c| c == "push:v1.0.0"));
    // crates.io delegation does NOT own the GitHub Release (that is cargo-dist's
    // narrower capability), so the coordinator still created it.
    assert!(tagger.calls().iter().any(|c| c == "release:v1.0.0"));
    // Green is observation-backed: the target was verified as Matches.
    assert_eq!(
        state.verified.get(&ids[0]),
        Some(&VerifyOutcome::Matches),
        "a delegated crates.io target must be observed on the index"
    );
    // The observation ran against the registry index, never the GitHub Release
    // asset observer (which would look in the wrong place entirely).
    assert!(
        !cmd.calls().iter().any(|c| c.starts_with("gh release view")),
        "a delegated crates.io target was verified as a GitHub Release: {:?}",
        cmd.calls()
    );
}

#[test]
fn a_ci_delegated_publish_that_never_lands_fails_the_cut_as_missing() {
    // CI silently not publishing (a broken/absent workflow, a bad secret) must end
    // the cut RED after the bounded wait — the mode must not degrade into "tagged,
    // assumed published".
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new().crate_version("1.0.0");
    let reg = FakeRegistry::empty();
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

    let plan = ci_publish_plan();
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
    let err = execute(&mut journal, &plan, &ctx, &tagger, &mut sink)
        .expect_err("an unobserved delegated publish must fail the cut");
    assert!(
        err.to_string().contains("missing at its destination"),
        "unexpected error: {err}"
    );
    let state = journal.state();
    assert_eq!(state.verified.get(&ids[0]), Some(&VerifyOutcome::Missing));
    assert!(state
        .phases
        .iter()
        .any(|r| r.phase == Phase::Verify && r.outcome == PhaseOutcome::Failed));
}

#[test]
fn a_registry_outage_leaves_a_delegated_publish_unknown_never_missing() {
    // The reconcile discipline: an outage is not evidence of absence. It still fails
    // the barrier (Unknown is not green), but with the honest outcome, so the
    // operator re-runs `release verify` instead of investigating a phantom CI bug.
    let clock = FakeClock(Cell::new(1000));
    let cmd = FakeCmd::new();
    let reg = UnreachableRegistry;
    let root = PathBuf::from("/repo");
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };
    let plan = ci_publish_plan();
    let target = AdapterTarget {
        target: crate::contract::schema::Target {
            ecosystem: Ecosystem::Rust,
            package: Some("tool".into()),
            registry: Registry::CratesIo,
            adapter: Adapter::CargoPublishCi,
        },
        package: "tool".into(),
        version: "1.0.0".into(),
    };
    assert_eq!(
        verify_delegated_registry(&ctx, &plan, &target),
        VerifyOutcome::Unknown
    );
}

#[test]
fn a_mixed_plan_publishes_the_engine_target_and_delegates_the_ci_one() {
    // Delegation is PER TARGET: a contract with one engine-published crate and one
    // CI-published crate must do both correctly in the same publish barrier.
    let store = FakeStore::default();
    let clock = FakeClock(Cell::new(1000));
    let idgen = FakeIdGen("RUN01".into());
    let cmd = FakeCmd::new().crate_version("1.0.0");
    // The engine's own publish lands in this map; the delegated crate is seeded as
    // already-on-the-index (CI published it), so both verify green.
    let reg = cmd.registry();
    reg.published
        .borrow_mut()
        .insert(("rust".into(), "tool".into()), "1.0.0".into());
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

    // An engine-published npm target alongside the CI-delegated crates.io one (two
    // ecosystems keeps the single-member fake workspace honest; the mixing that
    // matters is engine-owned vs delegated, not rust vs node).
    let plan = ReleasePlan {
        targets: vec![
            plan_target(Ecosystem::Node, Registry::Npm, Adapter::NpmPublish),
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublishCi),
        ],
        ..ci_publish_plan()
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
    execute(&mut journal, &plan, &ctx, &tagger, &mut sink).expect("a mixed cut completes");

    let state = journal.state();
    assert_eq!(state.status, RunStatus::Completed);
    assert!(state.published.contains_key(&ids[0]) && !state.delegated.contains(&ids[0]));
    assert!(state.delegated.contains(&ids[1]) && !state.published.contains_key(&ids[1]));
    // The engine-owned target really published; the delegated crate did not.
    assert!(
        cmd.calls().iter().any(|c| c.starts_with("npm publish")),
        "the engine-owned target was not published: {:?}",
        cmd.calls()
    );
    assert!(
        !cmd.calls()
            .iter()
            .any(|c| c.starts_with("cargo publish") && !c.contains("--dry-run")),
        "the delegated crate must not be published by the engine: {:?}",
        cmd.calls()
    );
    // Both are observed at their destination.
    assert_eq!(state.verified.get(&ids[0]), Some(&VerifyOutcome::Matches));
    assert_eq!(state.verified.get(&ids[1]), Some(&VerifyOutcome::Matches));
}
