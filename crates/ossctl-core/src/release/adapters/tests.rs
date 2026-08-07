//! Unit tests for the release adapter layer: registry dispatch to each
//! ecosystem, the `dry_run` command plans, `build`/`publish` driving the
//! injected runner, and `verify` classification across all four outcomes —
//! including the outage ⇒ `Unknown` path (never a false `Missing`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::path::Path;

use super::*;
use crate::contract::schema::{Adapter, Ecosystem, Registry, Target};
use crate::ports::{Clock, CommandOutput, CommandRunner, RegistryQuery};
use crate::protocol::release::{PublishReceipt, VerifyOutcome};

// ── Fakes ──────────────────────────────────────────────────────────────────

/// Recording command runner: returns success by default, records every call,
/// and can be told to fail a specific program or error on spawn.
struct FakeCmd {
    calls: RefCell<Vec<String>>,
    fail: Option<(String, i32, String)>,
    spawn_err: bool,
    /// Canned stdout served for `cargo metadata` (the workspace graph). `None`
    /// means empty output, which the adapter treats as a hard error (a real
    /// `cargo metadata` never emits empty stdout).
    metadata: Option<String>,
    /// Fail (non-zero exit) any call whose rendered form contains this substring —
    /// for the homebrew formula-existence probe, where a `404` must read as
    /// "absent" while sibling `gh`/`git` calls still succeed.
    fail_containing: Option<(String, i32, String)>,
    /// Serve this stdout for any call whose rendered form contains the key — e.g.
    /// the PR URL `gh pr create` prints.
    stdout_containing: Option<(String, String)>,
}

impl FakeCmd {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail: None,
            spawn_err: false,
            metadata: None,
            fail_containing: None,
            stdout_containing: None,
        }
    }
    /// Fail every call whose rendered form contains `needle` with `code`/`stderr`.
    fn fail_calls_containing(mut self, needle: &str, code: i32, stderr: &str) -> Self {
        self.fail_containing = Some((needle.to_string(), code, stderr.to_string()));
        self
    }
    /// Serve `stdout` for any call whose rendered form contains `needle`.
    fn stdout_calls_containing(mut self, needle: &str, stdout: &str) -> Self {
        self.stdout_containing = Some((needle.to_string(), stdout.to_string()));
        self
    }
    fn failing(program: &str, code: i32, stderr: &str) -> Self {
        Self {
            fail: Some((program.to_string(), code, stderr.to_string())),
            ..Self::new()
        }
    }
    fn spawn_error() -> Self {
        Self {
            spawn_err: true,
            ..Self::new()
        }
    }
    /// Serve `json` as the `cargo metadata` output so the adapter discovers a
    /// workspace graph.
    fn with_metadata(mut self, json: &str) -> Self {
        self.metadata = Some(json.to_string());
        self
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl CommandRunner for FakeCmd {
    fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
        let rendered = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(rendered.clone());
        if self.spawn_err {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        if let Some((needle, code, stderr)) = &self.fail_containing {
            if rendered.contains(needle) {
                return Ok(CommandOutput {
                    status: Some(*code),
                    stdout: String::new(),
                    stderr: stderr.clone(),
                });
            }
        }
        if let Some((needle, stdout)) = &self.stdout_containing {
            if rendered.contains(needle) {
                return Ok(CommandOutput {
                    status: Some(0),
                    stdout: stdout.clone(),
                    stderr: String::new(),
                });
            }
        }
        if let Some((p, code, stderr)) = &self.fail {
            if p == program {
                return Ok(CommandOutput {
                    status: Some(*code),
                    stdout: String::new(),
                    stderr: stderr.clone(),
                });
            }
        }
        if args.contains(&"metadata") {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: self.metadata.clone().unwrap_or_default(),
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

/// Fixed clock.
struct FakeClock(u64);
impl Clock for FakeClock {
    fn now_unix(&self) -> u64 {
        self.0
    }
}

/// A clock whose `sleep` advances virtual time instead of waiting, so a bounded
/// index-wait loop terminates instantly and deterministically under test.
struct AdvancingClock(Cell<u64>);
impl AdvancingClock {
    fn new() -> Self {
        Self(Cell::new(0))
    }
}
impl Clock for AdvancingClock {
    fn now_unix(&self) -> u64 {
        self.0.get()
    }
    fn sleep(&self, dur: Duration) {
        // Advance at least a second per sleep so a zero-second interval can never
        // stall the timeout comparison.
        self.0.set(self.0.get() + dur.as_secs().max(1));
    }
}

/// A registry where a package becomes visible only after `visible_after` polls —
/// exercises the index-wait poll/sleep loop reaching success mid-wait.
struct DelayedRegistry {
    package: String,
    version: String,
    visible_after: Cell<u32>,
}
impl DelayedRegistry {
    fn new(package: &str, version: &str, visible_after: u32) -> Self {
        Self {
            package: package.to_string(),
            version: version.to_string(),
            visible_after: Cell::new(visible_after),
        }
    }
}
impl RegistryQuery for DelayedRegistry {
    fn published_versions(&self, _ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        if package != self.package {
            return Ok(vec![]);
        }
        let remaining = self.visible_after.get();
        if remaining == 0 {
            return Ok(vec![self.version.clone()]);
        }
        self.visible_after.set(remaining - 1);
        Ok(vec![])
    }
}

/// A registry that errors for one specific package and reports the rest as
/// definitively absent — for the dep index-wait outage path, where the target's
/// own idempotency probe must succeed (`Ok(absent)`) while every poll for a
/// dependency fails.
struct ErrForRegistry {
    err_package: String,
}
impl ErrForRegistry {
    fn new(err_package: &str) -> Self {
        Self {
            err_package: err_package.to_string(),
        }
    }
}
impl RegistryQuery for ErrForRegistry {
    fn published_versions(&self, _ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        if package == self.err_package {
            Err(io::Error::from(io::ErrorKind::TimedOut))
        } else {
            Ok(Vec::new())
        }
    }
}

/// Programmable registry: maps `(ecosystem, package)` to a version list, or errors.
/// Records every lookup so a test can assert an index-independent phase (the build
/// phase) never queried it.
struct FakeRegistry {
    versions: HashMap<(String, String), Vec<String>>,
    err: bool,
    queries: RefCell<Vec<String>>,
}
impl FakeRegistry {
    fn new() -> Self {
        Self {
            versions: HashMap::new(),
            err: false,
            queries: RefCell::new(Vec::new()),
        }
    }
    fn with(mut self, ecosystem: &str, package: &str, versions: &[&str]) -> Self {
        self.versions.insert(
            (ecosystem.to_string(), package.to_string()),
            versions.iter().map(|s| (*s).to_string()).collect(),
        );
        self
    }
    fn erroring() -> Self {
        Self {
            err: true,
            ..Self::new()
        }
    }
    /// Every `(ecosystem, package)` lookup made, in call order.
    fn queries(&self) -> Vec<String> {
        self.queries.borrow().clone()
    }
}
impl RegistryQuery for FakeRegistry {
    fn published_versions(&self, ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        self.queries
            .borrow_mut()
            .push(format!("{ecosystem}:{package}"));
        if self.err {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        Ok(self
            .versions
            .get(&(ecosystem.to_string(), package.to_string()))
            .cloned()
            .unwrap_or_default())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn ctx<'a>(
    runner: &'a FakeCmd,
    clock: &'a FakeClock,
    registry: &'a FakeRegistry,
    root: &'a Path,
) -> EffectCtx<'a> {
    EffectCtx {
        runner,
        clock,
        registry,
        repo_root: root,
        artifacts: &EMPTY_ARTIFACTS,
    }
}

/// Like [`ctx`], but with concrete threaded [`ReleaseArtifacts`] — for exercising
/// the two distribution adapters that read `ctx.artifacts` in `publish`.
fn ctx_with<'a>(
    runner: &'a FakeCmd,
    clock: &'a FakeClock,
    registry: &'a FakeRegistry,
    root: &'a Path,
    artifacts: &'a ReleaseArtifacts,
) -> EffectCtx<'a> {
    EffectCtx {
        runner,
        clock,
        registry,
        repo_root: root,
        artifacts,
    }
}

/// Like [`ctx`], but with an [`AdvancingClock`] and a trait-object registry — for
/// the multi-crate publish path, which polls the registry and sleeps between
/// index checks.
fn ctx_advancing<'a>(
    runner: &'a FakeCmd,
    clock: &'a AdvancingClock,
    registry: &'a dyn RegistryQuery,
    root: &'a Path,
) -> EffectCtx<'a> {
    EffectCtx {
        runner,
        clock,
        registry,
        repo_root: root,
        artifacts: &EMPTY_ARTIFACTS,
    }
}

fn target(
    ecosystem: Ecosystem,
    registry: Registry,
    adapter: Adapter,
    version: &str,
) -> AdapterTarget {
    target_named(ecosystem, registry, adapter, "tool", version)
}

/// Like [`target`], but with an explicit package name — the cargo workspace path
/// requires the target package to be a real publishable member.
fn target_named(
    ecosystem: Ecosystem,
    registry: Registry,
    adapter: Adapter,
    package: &str,
    version: &str,
) -> AdapterTarget {
    AdapterTarget {
        target: Target {
            ecosystem,
            package: Some(package.to_string()),
            registry,
            adapter,
        },
        package: package.to_string(),
        version: version.to_string(),
    }
}

/// `cargo metadata` JSON for a single-crate workspace named `name`@`version`.
fn metadata_single(name: &str, version: &str) -> String {
    let id = format!("{name} {version}");
    format!(
        r#"{{"packages":[{{"name":"{name}","version":"{version}","id":"{id}","dependencies":[],"publish":null}}],"workspace_members":["{id}"]}}"#
    )
}

/// `cargo metadata` JSON for a two-crate workspace where `bin` depends on `lib`
/// (and `lib` dev-depends back on `bin` — a legitimate cycle the ordering must
/// ignore). Both share `version`.
fn metadata_two_crate(lib: &str, bin: &str, version: &str) -> String {
    let lib_id = format!("{lib} {version}");
    let bin_id = format!("{bin} {version}");
    format!(
        r#"{{"packages":[
            {{"name":"{bin}","version":"{version}","id":"{bin_id}","publish":null,
              "dependencies":[{{"name":"{lib}","kind":null}}]}},
            {{"name":"{lib}","version":"{version}","id":"{lib_id}","publish":null,
              "dependencies":[{{"name":"{bin}","kind":"dev"}}]}}
        ],"workspace_members":["{lib_id}","{bin_id}"]}}"#
    )
}

fn receipt(ecosystem: Ecosystem, version: &str, digest: Option<&str>) -> PublishReceipt {
    PublishReceipt {
        adapter: Adapter::CargoPublish,
        ecosystem,
        package: "tool".to_string(),
        version: version.to_string(),
        canonical_ref: format!("crates.io/tool@{version}"),
        digest: digest.map(str::to_string),
        remote_url: None,
        timestamp: 100,
    }
}

// ── Registry dispatch: every adapter identity wires to an ecosystem ──────────

#[test]
fn resolve_dispatches_every_adapter_identity() {
    let cases = [
        (Adapter::CargoPublish, "rust"),
        (Adapter::CargoDist, "rust"),
        (Adapter::ReleasePlease, "node"),
        (Adapter::Changesets, "node"),
        (Adapter::NpmPublish, "node"),
        (Adapter::GhActionPypiPublish, "python"),
        (Adapter::Twine, "python"),
        (Adapter::Goreleaser, "go"),
        (Adapter::HomebrewTap, "homebrew"),
        (Adapter::HomebrewCore, "homebrew"),
        (Adapter::Manual, "binary"),
    ];
    for (id, family) in cases {
        let resolved = resolve(id);
        // The resolved adapter reports back the identity it operates as …
        assert_eq!(resolved.adapter(), id, "identity round-trips for {id:?}");
        // … and routes to the expected ecosystem family variant.
        let actual = match resolved {
            EcosystemAdapter::Rust(_) => "rust",
            EcosystemAdapter::Node(_) => "node",
            EcosystemAdapter::Python(_) => "python",
            EcosystemAdapter::Go(_) => "go",
            EcosystemAdapter::Homebrew(_) => "homebrew",
            EcosystemAdapter::Binary(_) => "binary",
        };
        assert_eq!(actual, family, "{id:?} routes to {family}");
    }
}

#[test]
fn every_adapter_has_a_nonzero_timeout() {
    for id in Adapter::VALID.iter().filter_map(|s| Adapter::parse(s)) {
        assert!(
            resolve(id).timeout() > Duration::ZERO,
            "{id:?} must declare a timeout"
        );
    }
}

// ── dry_run produces real, side-effect-free command plans ────────────────────

#[test]
fn dry_run_runs_the_index_independent_build_gate_as_preflight() {
    let cmd = FakeCmd::new().with_metadata(&metadata_single("tool", "1.2.3"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "1.2.3",
    );
    let report = resolve(Adapter::CargoPublish).dry_run(&c, &t).unwrap();

    assert_eq!(report.adapter, Adapter::CargoPublish);
    // The preview lists the same index-independent gate build-all runs: a local
    // `cargo check` then a `--no-verify` package.
    let rendered: Vec<String> = report
        .planned_commands
        .iter()
        .map(crate::protocol::release::PlannedCommand::rendered)
        .collect();
    assert_eq!(
        rendered,
        vec![
            "cargo check -p tool".to_string(),
            "cargo package --registry crates-io -p tool --no-verify".to_string(),
        ]
    );
    // dry_run is a FAITHFUL PREFLIGHT: it reads the workspace graph (read-only) and
    // then actually runs that same gate, so a plan that cannot compile or package
    // fails here — not mid-cut. No `cargo publish` and no `--verify` compile, so it
    // never touches the registry index.
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p tool",
            "cargo package --registry crates-io -p tool --no-verify",
        ]
    );
    assert!(
        !cmd.calls().iter().any(|call| call.contains("publish")),
        "dry-run executed a publish: {:?}",
        cmd.calls()
    );
    // The publish command a real cut runs is still surfaced as a note.
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("cargo publish --registry crates-io -p tool")),
        "dry-run dropped the publish note: {:?}",
        report.notes
    );
}

#[test]
fn dry_run_available_for_every_ecosystem() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    for (eco, registry, id) in [
        (Ecosystem::Rust, Registry::CratesIo, Adapter::CargoDist),
        (Ecosystem::Node, Registry::Npm, Adapter::NpmPublish),
        (Ecosystem::Node, Registry::Npm, Adapter::ReleasePlease),
        (Ecosystem::Python, Registry::Pypi, Adapter::Twine),
        (
            Ecosystem::Python,
            Registry::Pypi,
            Adapter::GhActionPypiPublish,
        ),
        (Ecosystem::Go, Registry::ProxyGolangOrg, Adapter::Goreleaser),
        (Ecosystem::Binary, Registry::Homebrew, Adapter::HomebrewTap),
        (Ecosystem::Binary, Registry::GhReleases, Adapter::Manual),
    ] {
        let t = target(eco, registry, id, "1.0.0");
        let report = resolve(id).dry_run(&c, &t).unwrap();
        assert_eq!(report.adapter, id);
        assert!(
            !report.planned_commands.is_empty(),
            "{id:?} dry_run planned no commands"
        );
    }
    assert!(cmd.calls().is_empty(), "a dry_run executed a command");
}

// ── build / publish drive the injected runner ────────────────────────────────

#[test]
fn publish_runs_the_registry_command_and_returns_a_receipt() {
    let cmd = FakeCmd::new().with_metadata(&metadata_single("tool", "2.0.0"));
    let clock = FakeClock(42);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "2.0.0",
    );
    let r = resolve(Adapter::CargoPublish).publish(&c, &t).unwrap();

    // A single-crate workspace publishes exactly once, with no index-wait after.
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo publish --registry crates-io -p tool"
        ]
    );
    assert_eq!(r.adapter, Adapter::CargoPublish);
    assert_eq!(r.ecosystem, Ecosystem::Rust);
    assert_eq!(r.package, "tool");
    assert_eq!(r.version, "2.0.0");
    assert_eq!(r.canonical_ref, "crates.io/tool@2.0.0");
    assert_eq!(r.timestamp, 42, "receipt stamps the injected clock");
    assert_eq!(
        r.remote_url.as_deref(),
        Some("https://crates.io/crates/tool/2.0.0")
    );
}

#[test]
fn cargo_build_packages_a_leaf_crate() {
    // A LEAF crate (no publishable workspace deps) is fully packaged in build-all: a
    // local `cargo check` (the pre-publish compile safety net — catches real compile
    // errors before any publish, resolving via the sibling `path` not the index) then
    // a `cargo package` carrying `--registry crates-io` (same registry the publish
    // phase targets, never ambient config) AND `--no-verify` (skips the isolated
    // verify compile, redundant with the check above and re-run by `cargo publish`).
    // A read-only `cargo metadata` runs first to classify the target as a leaf.
    let cmd = FakeCmd::new().with_metadata(&metadata_single("tool", "1.2.3"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "1.2.3",
    );
    let b = resolve(Adapter::CargoPublish).build(&c, &t).unwrap();

    assert_eq!(b.adapter, Adapter::CargoPublish);
    // The `.crate` artifact is recorded — a leaf packages under `--no-verify`.
    assert_eq!(b.artifacts, vec!["tool-1.2.3.crate".to_string()]);
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p tool",
            "cargo package --registry crates-io -p tool --no-verify",
        ]
    );
    // The build phase must never query the registry index — the check resolves via
    // the local path and `--no-verify` skips the index-resolving verify.
    assert!(reg.queries().is_empty(), "build queried the registry index");
}

#[test]
fn cargo_build_defers_packaging_for_a_dependent_on_an_unpublished_crate() {
    // A DEPENDENT crate (`bin` pins `lib = "=X.Y.Z"`) whose dependency `lib` is NOT
    // yet on the crates.io index CANNOT be `cargo package`d (packaging resolves the
    // pinned dep against the index even with `--no-verify`), so its packaging is
    // DEFERRED to `cargo publish` in publish-all. build runs only the
    // index-independent `cargo check` gate and records NO `.crate` artifact. It DOES
    // probe the registry (read-only) to classify the target, but never runs a
    // `cargo package` that would touch the index.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "0.2.0"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::new(); // empty: `lib` not published yet
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "0.2.0",
    );
    let b = resolve(Adapter::CargoPublish).build(&c, &t).unwrap();

    // Only the compile gate ran; no `cargo package`, and no `.crate` artifact.
    assert!(
        b.artifacts.is_empty(),
        "a deferred build produced an artifact"
    );
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p bin",
        ]
    );
    // A note records that packaging is deferred (operator-facing observability).
    assert!(
        b.notes.iter().any(|n| n.contains("deferred")),
        "build dropped the deferred-packaging note: {:?}",
        b.notes
    );
    // Classification probes the registry for the dep once; no `cargo package` runs.
    assert_eq!(reg.queries(), vec!["rust:lib".to_string()]);
}

#[test]
fn cargo_build_packages_a_dependent_whose_dep_is_already_published() {
    // A DEPENDENT crate `bin` whose dependency `lib` IS ALREADY on the crates.io
    // index (a re-cut where only `bin` changed) CAN be packaged — `cargo package`
    // resolves the `=`-pinned dep against the index it is already on — so build does
    // NOT defer: it packages `bin` and records the `.crate`, preserving the manifest-
    // validation safety net. This is the registry-aware refinement: defer only when a
    // workspace dep is not yet on the index, never merely because an edge exists.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "0.2.0"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::new().with("rust", "lib", &["0.2.0"]); // lib already published
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "0.2.0",
    );
    let b = resolve(Adapter::CargoPublish).build(&c, &t).unwrap();

    // The dependent packages (its dep is already indexed), producing the `.crate`.
    assert_eq!(b.artifacts, vec!["bin-0.2.0.crate".to_string()]);
    assert!(
        b.notes.is_empty(),
        "an already-packageable dependent should not carry a deferral note: {:?}",
        b.notes
    );
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p bin",
            "cargo package --registry crates-io -p bin --no-verify",
        ]
    );
    // It probed the registry for `lib` to make the decision.
    assert_eq!(reg.queries(), vec!["rust:lib".to_string()]);
}

#[test]
fn build_phase_clears_both_crates_when_a_pinned_dep_is_not_yet_published() {
    // Done criterion #1 (`release-cut-build-phase-dep-ordering`): a two-crate
    // workspace where `bin` pins `lib = "=X.Y.Z"` must clear the BUILD phase even
    // though `lib` is not on the crates.io index yet — it is only *published* later,
    // in publish-all. The leaf `lib` packages now (`cargo package --no-verify`); the
    // dependent `bin` CANNOT be packaged pre-publish, so build runs its `cargo check`
    // gate only and defers packaging to `cargo publish`. Both clear the phase with an
    // EMPTY registry, and neither runs a `cargo package` that touches the index (the
    // dependent's only registry contact is the read-only classification probe).
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "0.2.0"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::new(); // empty: `lib` is NOT published yet
    let root = Path::new("/repo");

    // The leaf `lib` packages; the dependent `bin` defers packaging.
    let c = ctx(&cmd, &clock, &reg, root);
    let lib = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "lib",
        "0.2.0",
    );
    let lib_build = resolve(Adapter::CargoPublish).build(&c, &lib).unwrap();
    assert_eq!(lib_build.artifacts, vec!["lib-0.2.0.crate".to_string()]);

    let c = ctx(&cmd, &clock, &reg, root);
    let bin = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "0.2.0",
    );
    let bin_build = resolve(Adapter::CargoPublish).build(&c, &bin).unwrap();
    assert!(
        bin_build.artifacts.is_empty(),
        "the dependent packaged in build-all instead of deferring"
    );

    // The leaf cleared check + `--no-verify` package; the dependent cleared its
    // `cargo check` gate only; no `cargo package` for `bin` (which would resolve its
    // pinned dep against the index).
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p lib",
            "cargo package --registry crates-io -p lib --no-verify",
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p bin",
        ]
    );
    // The leaf (no workspace deps) never probes; the dependent probes `lib` once to
    // classify. No `cargo package` for the dependent, so nothing resolves against the
    // index.
    assert_eq!(reg.queries(), vec!["rust:lib".to_string()]);
}

#[test]
fn dry_run_preflights_a_dependent_without_the_pinned_dep_on_the_index() {
    // Done criterion #2 (positive half): the dry-run preflight of the DEPENDENT
    // crate must NOT false-fail just because its `=`-pinned workspace dep is not on
    // the index yet. Because that dependent cannot be `cargo package`d pre-publish,
    // the preflight is the index-independent `cargo check` compile gate ALONE (no
    // package), so an empty registry still passes — dry-run validates what it *can*
    // pre-publish and defers packaging to the real `cargo publish`.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "0.2.0"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::new(); // empty: `lib` not published yet
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "0.2.0",
    );
    let report = resolve(Adapter::CargoPublish).dry_run(&c, &t).unwrap();

    // The preflight ran the read-only graph query then the compile gate only (no
    // `cargo package` for the dependent, so nothing resolves against the index).
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p bin",
        ]
    );
    // It probed `lib` once (read-only) to classify the target as deferring.
    assert_eq!(reg.queries(), vec!["rust:lib".to_string()]);
    // The notes surface both that packaging is deferred and the dependency the real
    // cut will wait on.
    assert!(
        report.notes.iter().any(|n| n.contains("deferred")),
        "dry-run dropped the deferred-packaging note: {:?}",
        report.notes
    );
    assert!(
        report.notes.iter().any(|n| n.contains("lib@0.2.0")),
        "dry-run dropped the workspace-dep wait note: {:?}",
        report.notes
    );
}

#[test]
fn dry_run_fails_when_the_package_preflight_fails() {
    // Done criterion #2 (negative half): a genuinely-unbuildable plan must fail at
    // DRY-RUN-ALL — before any external effect — not mid-cut. The `cargo package`
    // preflight failing propagates as a dry-run error.
    let cmd = FakeCmd::new()
        .with_metadata(&metadata_single("tool", "1.2.3"))
        .fail_calls_containing("cargo package", 101, "error: could not compile `tool`");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "1.2.3",
    );
    let err = resolve(Adapter::CargoPublish).dry_run(&c, &t).unwrap_err();
    match err {
        AdapterError::Command { code, .. } => assert_eq!(code, Some(101)),
        other => panic!("expected a Command error from the package preflight, got {other:?}"),
    }
}

#[test]
fn build_phase_fails_on_a_genuine_compile_error_before_any_publish() {
    // Done criterion #3 (the important half): a crate that genuinely does not COMPILE
    // must fail the BUILD phase — not slip through `--no-verify` packaging and only
    // fail later in publish-all's `cargo publish`, AFTER the dependency crate is
    // already irreversibly published (the partial-publish trap). The index-independent
    // `cargo check` gate catches it in build-all, before any publish.
    let cmd = FakeCmd::new()
        .with_metadata(&metadata_single("tool", "1.2.3"))
        .fail_calls_containing("cargo check", 101, "error[E0308]: mismatched types");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "1.2.3",
    );
    let err = resolve(Adapter::CargoPublish).build(&c, &t).unwrap_err();
    match err {
        AdapterError::Command { code, .. } => assert_eq!(code, Some(101)),
        other => panic!("expected a Command error from the failing compile, got {other:?}"),
    }
    // It failed on the compile gate — packaging was never reached (metadata classifies
    // the target first, then the `cargo check` gate fails).
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p tool",
        ]
    );
}

#[test]
fn build_phase_propagates_a_real_package_failure() {
    // Done criterion #3 (packaging half): for a LEAF crate that IS packaged in
    // build-all, a packaging failure (e.g. a crate that compiles but cannot package —
    // a missing included file, a bad manifest) must still fail the BUILD phase;
    // `--no-verify` must not turn `build` into a no-op that swallows a failing
    // `cargo package`.
    let cmd = FakeCmd::new()
        .with_metadata(&metadata_single("tool", "1.2.3"))
        .fail_calls_containing(
            "cargo package",
            101,
            "error: invalid inclusion of reserved file name",
        );
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "1.2.3",
    );
    let err = resolve(Adapter::CargoPublish).build(&c, &t).unwrap_err();
    match err {
        AdapterError::Command { code, .. } => assert_eq!(code, Some(101)),
        other => panic!("expected a Command error from the failing package, got {other:?}"),
    }
    // The compile gate passed; packaging is where it failed (a leaf packages, so the
    // failure is caught in build-all, before any publish).
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo check -p tool",
            "cargo package --registry crates-io -p tool --no-verify",
        ]
    );
}

#[test]
fn cargo_rejects_a_non_crates_io_registry_before_any_action() {
    // crates.io is the only rust registry ossctl supports. A cargo-publish target
    // pointed at any other registry is a misconfiguration that must fail fast with a
    // typed error — never shell out (which could publish to the wrong destination).
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");

    // `Registry::Npm` stands in for any non-crates.io registry a rust target could be
    // misconfigured with; the guard keys off `registry != CratesIo`, not the value.
    let t = target(
        Ecosystem::Rust,
        Registry::Npm,
        Adapter::CargoPublish,
        "1.0.0",
    );

    // dry_run, build, and publish each reject the target up front, running NO command.
    for op in ["dry_run", "build", "publish"] {
        let cmd = FakeCmd::new();
        let c = ctx(&cmd, &clock, &reg, root);
        let adapter = resolve(Adapter::CargoPublish);
        let err = match op {
            "dry_run" => adapter.dry_run(&c, &t).unwrap_err(),
            "build" => adapter.build(&c, &t).unwrap_err(),
            _ => adapter.publish(&c, &t).unwrap_err(),
        };
        match err {
            AdapterError::UnsupportedRegistry { adapter, registry } => {
                assert_eq!(adapter, Adapter::CargoPublish);
                assert_eq!(registry, Registry::Npm);
            }
            other => panic!("{op}: expected UnsupportedRegistry, got {other:?}"),
        }
        assert!(
            cmd.calls().is_empty(),
            "{op} shelled out before rejecting a non-crates.io registry: {:?}",
            cmd.calls()
        );
    }
}

#[test]
fn publish_propagates_a_command_failure() {
    let cmd = FakeCmd::failing("npm", 1, "402 Payment Required");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(Ecosystem::Node, Registry::Npm, Adapter::NpmPublish, "1.0.0");
    let err = resolve(Adapter::NpmPublish).publish(&c, &t).unwrap_err();
    match err {
        AdapterError::Command { code, .. } => assert_eq!(code, Some(1)),
        other => panic!("expected Command error, got {other:?}"),
    }
}

#[test]
fn publish_surfaces_a_spawn_failure_as_io_error() {
    let cmd = FakeCmd::spawn_error();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(Ecosystem::Node, Registry::Npm, Adapter::NpmPublish, "1.0.0");
    let err = resolve(Adapter::NpmPublish).publish(&c, &t).unwrap_err();
    assert!(matches!(err, AdapterError::Io { .. }));
}

#[test]
fn cargo_dist_publish_is_unsupported_from_host() {
    // cargo-dist only builds locally; its upload is the CI workflow, so it must
    // not fabricate a receipt for a publish that did not happen.
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoDist,
        "1.0.0",
    );
    let err = resolve(Adapter::CargoDist).publish(&c, &t).unwrap_err();
    assert!(matches!(
        err,
        AdapterError::Unsupported {
            operation: "publish",
            ..
        }
    ));
    assert!(cmd.calls().is_empty());
}

#[test]
fn release_please_publish_is_unsupported_from_host() {
    // release-please publishes on merge via a CI job keyed off the GitHub release;
    // there is no faithful host publish, so it must report Unsupported rather than
    // run a representative command and fabricate an npm receipt.
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Node,
        Registry::Npm,
        Adapter::ReleasePlease,
        "1.0.0",
    );
    let err = resolve(Adapter::ReleasePlease).publish(&c, &t).unwrap_err();
    assert!(
        matches!(
            err,
            AdapterError::Unsupported {
                adapter: Adapter::ReleasePlease,
                operation: "publish",
            }
        ),
        "expected Unsupported publish for release-please, got {err:?}"
    );
    assert!(
        cmd.calls().is_empty(),
        "an unsupported publish must run no command"
    );
}

#[test]
fn npm_publish_runs_the_real_publish_and_returns_a_receipt() {
    // Guards the REAL npm path: `npm publish` runs and the receipt records the
    // npmjs.com URL for the published version.
    let cmd = FakeCmd::new();
    let clock = FakeClock(7);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(Ecosystem::Node, Registry::Npm, Adapter::NpmPublish, "1.0.0");
    let r = resolve(Adapter::NpmPublish).publish(&c, &t).unwrap();
    assert_eq!(cmd.calls(), vec!["npm publish".to_string()]);
    assert_eq!(r.package, "tool");
    assert_eq!(r.version, "1.0.0");
    assert_eq!(
        r.remote_url.as_deref(),
        Some("https://www.npmjs.com/package/tool/v/1.0.0")
    );
}

#[test]
fn changeset_publish_runs_the_real_publish() {
    // Guards the REAL changesets path: `changeset publish` runs.
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(Ecosystem::Node, Registry::Npm, Adapter::Changesets, "1.0.0");
    resolve(Adapter::Changesets).publish(&c, &t).unwrap();
    assert_eq!(cmd.calls(), vec!["changeset publish".to_string()]);
}

#[test]
fn node_build_reads_the_real_tarball_name_from_npm_pack_json() {
    // A scoped package `@scope/pkg` packs to `scope-pkg-{ver}.tgz`, not
    // `{pkg}-{ver}.tgz`; build must report the name npm actually produced so the
    // asset threaded to the binary upload set is correct.
    let cmd = FakeCmd::new().stdout_calls_containing(
        "pack",
        r#"[{"filename":"scope-pkg-1.0.0.tgz","name":"@scope/pkg","version":"1.0.0"}]"#,
    );
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Node,
        Registry::Npm,
        Adapter::NpmPublish,
        "@scope/pkg",
        "1.0.0",
    );
    let b = resolve(Adapter::NpmPublish).build(&c, &t).unwrap();
    assert_eq!(cmd.calls(), vec!["npm pack --json".to_string()]);
    assert_eq!(b.artifacts, vec!["scope-pkg-1.0.0.tgz".to_string()]);
    assert!(b.notes.is_empty());
}

#[test]
fn node_build_errors_when_npm_pack_emits_no_parseable_json() {
    // A build that cannot read npm's reported filename must fail hard rather than
    // guess a `{package}-{version}.tgz` name — that reconstruction is wrong for
    // scoped packages and would only surface as an opaque upload failure later.
    // (`FakeCmd::new()` serves empty stdout for `npm pack --json`.)
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(Ecosystem::Node, Registry::Npm, Adapter::NpmPublish, "1.0.0");
    let err = resolve(Adapter::NpmPublish).build(&c, &t).unwrap_err();
    assert!(
        matches!(err, AdapterError::Command { code: None, .. }),
        "expected a hard Command error on unparseable npm pack output, got {err:?}"
    );
}

#[test]
fn node_build_errors_on_an_empty_npm_pack_json_array() {
    // An empty JSON array parses cleanly but names no artifact — still a hard error,
    // never a fabricated filename.
    let cmd = FakeCmd::new().stdout_calls_containing("pack", "[]");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(Ecosystem::Node, Registry::Npm, Adapter::NpmPublish, "1.0.0");
    let err = resolve(Adapter::NpmPublish).build(&c, &t).unwrap_err();
    assert!(matches!(err, AdapterError::Command { code: None, .. }));
}

// ── One target = one publish unit: dep index-wait, no cross-target publish ───

#[test]
fn target_waits_for_its_workspace_deps_before_publishing_only_its_own_crate() {
    // ADR-0004: publishing the `bin` target must publish ONLY `bin` — never `lib`,
    // which is `lib`'s own target (cut earlier by the coordinator). `bin` depends
    // on `lib`, so it first waits for `lib` to be crates.io-index-visible; `lib` is
    // visible only on a later poll, exercising a real (bounded) wait that succeeds
    // mid-loop after sleeping.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "1.0.0"));
    let clock = AdvancingClock::new();
    // visible_after=2: `bin`'s own idempotency probe hits `lib`'s package name only
    // via the dep-wait; `lib` appears after a couple of polls.
    let reg = DelayedRegistry::new("lib", "1.0.0", 2);
    let root = Path::new("/repo");
    let c = ctx_advancing(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "1.0.0",
    );
    let r = resolve(Adapter::CargoPublish).publish(&c, &t).unwrap();

    // Waits for `lib` (never re-publishing it), then publishes only `bin`.
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo publish --registry crates-io -p bin",
        ]
    );
    // The wait actually slept (advanced the virtual clock) before lib became
    // visible.
    assert!(clock.now_unix() > 0, "the index-wait never polled/slept");
    // The receipt names the target's own package.
    assert_eq!(r.package, "bin");
    assert_eq!(r.version, "1.0.0");
}

#[test]
fn target_without_workspace_deps_publishes_immediately() {
    // A `lib` target (no publishable workspace deps of its own) is the dependency,
    // cut first by the coordinator. It publishes with no index-wait — regardless of
    // any dependent still to come (the dependent's target owns that wait, ADR-0004).
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "1.0.0"));
    let clock = AdvancingClock::new();
    let reg = FakeRegistry::new(); // empty — lib not yet published
    let root = Path::new("/repo");
    let c = ctx_advancing(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "lib",
        "1.0.0",
    );
    resolve(Adapter::CargoPublish).publish(&c, &t).unwrap();

    // Exactly one publish of `lib`, and no wait (lib has no publishable workspace
    // dependency of its own).
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo publish --registry crates-io -p lib",
        ]
    );
    assert_eq!(
        clock.now_unix(),
        0,
        "an independent crate must not index-wait"
    );
}

#[test]
fn two_targets_never_double_publish_the_shared_dependency() {
    // The double-publish regression (release-cut-multi-target-ecosystem): two
    // crates.io targets `lib` then `bin` (bin depends on lib). Publishing them in
    // coordinator dependency order over ONE shared registry view must run
    // `cargo publish -p lib` EXACTLY ONCE — even though `lib` is still index-lagged
    // when `bin`'s target publishes. `bin` waits for `lib`, it never re-publishes it.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "1.0.0"));
    let clock = AdvancingClock::new();
    // `lib` becomes visible only after a couple of polls — the publish→index lag
    // window in which the old closure model re-ran `cargo publish -p lib`.
    let reg = DelayedRegistry::new("lib", "1.0.0", 2);
    let root = Path::new("/repo");
    let c = ctx_advancing(&cmd, &clock, &reg, root);

    // Target 1: the dependency `lib` (no deps) — publishes immediately.
    let lib_t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "lib",
        "1.0.0",
    );
    resolve(Adapter::CargoPublish).publish(&c, &lib_t).unwrap();

    // Target 2: the dependent `bin` — waits for lib to index, then publishes bin.
    let bin_t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "1.0.0",
    );
    resolve(Adapter::CargoPublish).publish(&c, &bin_t).unwrap();

    // `lib` was published exactly once (by its own target); `bin` once.
    assert_eq!(
        cmd.calls()
            .iter()
            .filter(|c| *c == "cargo publish --registry crates-io -p lib")
            .count(),
        1,
        "the shared dependency was published more than once: {:?}",
        cmd.calls()
    );
    assert_eq!(
        cmd.calls()
            .iter()
            .filter(|c| *c == "cargo publish --registry crates-io -p bin")
            .count(),
        1
    );
}

#[test]
fn target_skips_its_own_publish_when_already_published_on_resume() {
    // Idempotent re-entry: `bin`'s version is already on the index (a prior attempt
    // landed it), so the adapter must NOT re-run `cargo publish -p bin` — that would
    // hard-fail on crates.io and wedge the resume. The pre-publish probe short-
    // circuits before any command runs.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "1.0.0"));
    let clock = AdvancingClock::new();
    let reg = FakeRegistry::new()
        .with("rust", "lib", &["1.0.0"])
        .with("rust", "bin", &["1.0.0"]);
    let root = Path::new("/repo");
    let c = ctx_advancing(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "1.0.0",
    );
    resolve(Adapter::CargoPublish).publish(&c, &t).unwrap();

    // Already published ⇒ no command ran at all (not even the metadata probe — the
    // registry probe answered first).
    assert!(
        cmd.calls().is_empty(),
        "an already-published target must run no command: {:?}",
        cmd.calls()
    );
}

#[test]
fn target_index_wait_times_out_and_never_publishes_its_crate() {
    // `bin`'s dependency `lib` never becomes index-visible, so the wait exhausts the
    // timeout — and `bin` must NOT publish (crates.io would reject it, a partial
    // publish). No `cargo publish -p bin` runs.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "1.0.0"));
    let clock = AdvancingClock::new();
    let reg = FakeRegistry::new(); // empty — lib never appears
    let root = Path::new("/repo");
    let c = ctx_advancing(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "1.0.0",
    );
    let err = resolve(Adapter::CargoPublish).publish(&c, &t).unwrap_err();

    match err {
        AdapterError::IndexTimeout {
            package, version, ..
        } => {
            assert_eq!(package, "lib");
            assert_eq!(version, "1.0.0");
        }
        other => panic!("expected IndexTimeout, got {other:?}"),
    }
    // The timeout stopped the cut before `bin` published.
    assert!(
        !cmd.calls().iter().any(|call| call.contains("publish")),
        "the dependent published despite a dep index-timeout: {:?}",
        cmd.calls()
    );
}

#[test]
fn publish_fails_closed_when_the_registry_is_unreachable() {
    // TRI-STATE fail-closed (ADR-0004): the pre-publish probe cannot reach the
    // registry, so it cannot prove `tool@1.0.0` has NOT already landed. The publish
    // must abort with `RegistryUnavailable` and run NO `cargo publish` — never
    // treat an outage as "not published" and risk a duplicate, irreversible upload.
    let cmd = FakeCmd::new().with_metadata(&metadata_single("tool", "1.0.0"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::erroring();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "1.0.0",
    );
    let err = resolve(Adapter::CargoPublish).publish(&c, &t).unwrap_err();
    match err {
        AdapterError::RegistryUnavailable {
            package, version, ..
        } => {
            assert_eq!(package, "tool");
            assert_eq!(version, "1.0.0");
        }
        other => panic!("expected RegistryUnavailable, got {other:?}"),
    }
    // Nothing published — the probe fails closed before the workspace is even read.
    assert!(
        cmd.calls().is_empty(),
        "a fail-closed publish ran a command: {:?}",
        cmd.calls()
    );
}

#[test]
fn dep_index_wait_reports_registry_unavailable_not_a_false_index_timeout() {
    // The dep index-wait can never reach the registry to confirm `lib` is visible
    // (every poll errors). On timeout it must surface RegistryUnavailable — an
    // honest outage — NOT a misleading IndexTimeout ("did not appear ... after
    // publishing"). `bin`'s own probe still answers (Ok absent), so the wait is
    // reached; `bin` must not publish.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "1.0.0"));
    let clock = AdvancingClock::new();
    let reg = ErrForRegistry::new("lib");
    let root = Path::new("/repo");
    let c = ctx_advancing(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "1.0.0",
    );
    let err = resolve(Adapter::CargoPublish).publish(&c, &t).unwrap_err();
    match err {
        AdapterError::RegistryUnavailable {
            package, version, ..
        } => {
            assert_eq!(package, "lib");
            assert_eq!(version, "1.0.0");
        }
        other => panic!("expected RegistryUnavailable, got {other:?}"),
    }
    assert!(
        !cmd.calls().iter().any(|call| call.contains("publish")),
        "the dependent published despite an unreachable registry: {:?}",
        cmd.calls()
    );
}

#[test]
fn target_dry_run_reports_its_own_publish_and_the_dep_wait() {
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "1.0.0"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "1.0.0",
    );
    let report = resolve(Adapter::CargoPublish).dry_run(&c, &t).unwrap();

    // The preflight gate for the target's own crate: a `=`-pinned DEPENDENT defers
    // packaging, so the gate is the `cargo check` compile safety net alone (exactly
    // what build-all runs for it), never another target's crate and never a package.
    let rendered: Vec<String> = report
        .planned_commands
        .iter()
        .map(crate::protocol::release::PlannedCommand::rendered)
        .collect();
    assert_eq!(rendered, vec!["cargo check -p bin".to_string()]);
    // The note names the workspace dependency the real cut waits to index first.
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("lib@1.0.0") && n.contains("bin")),
        "notes missing the dep index-wait: {:?}",
        report.notes
    );
    // dry_run is a faithful preflight, not a real publish: it runs the read-only
    // metadata query and the compile gate, but never `cargo publish` or `cargo package`.
    assert!(
        !cmd.calls().iter().any(|call| call.contains("publish")),
        "dry_run executed a publish: {:?}",
        cmd.calls()
    );
    assert!(
        !cmd.calls().iter().any(|call| call.contains("package")),
        "dry_run packaged a dependent that must defer: {:?}",
        cmd.calls()
    );
}

#[test]
fn workspace_target_package_must_be_a_publishable_member() {
    // The contract's package is not a member of the workspace graph → hard error
    // rather than silently publishing something else.
    let cmd = FakeCmd::new().with_metadata(&metadata_two_crate("lib", "bin", "1.0.0"));
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "ghost",
        "1.0.0",
    );
    let err = resolve(Adapter::CargoPublish).publish(&c, &t).unwrap_err();
    match err {
        AdapterError::Command { stderr, .. } => {
            assert!(stderr.contains("ghost"), "message should name the package");
        }
        other => panic!("expected Command error, got {other:?}"),
    }
    // Nothing was published.
    assert!(!cmd.calls().iter().any(|call| call.contains("publish")));
}

#[test]
fn target_only_waits_for_crates_io_publishable_deps() {
    // `bin` depends on `lib` (crates.io), `internal` (restricted to another
    // registry), and `helper` (`publish = false`). Only `lib` is a crates.io
    // dependency, so the target waits for `lib` alone before publishing `bin`; it
    // must NOT wait for `internal`/`helper` (they will never appear on crates.io,
    // so a wait on them would time out). `lib` is visible after one poll.
    let meta = r#"{"packages":[
        {"name":"bin","version":"1.0.0","id":"bin 1.0.0","publish":null,
          "dependencies":[
            {"name":"lib","kind":null},
            {"name":"internal","kind":null},
            {"name":"helper","kind":null}
          ]},
        {"name":"lib","version":"1.0.0","id":"lib 1.0.0","publish":null,"dependencies":[]},
        {"name":"helper","version":"1.0.0","id":"helper 1.0.0","publish":[],"dependencies":[]},
        {"name":"internal","version":"1.0.0","id":"internal 1.0.0","publish":["other-reg"],"dependencies":[]}
    ],"workspace_members":["bin 1.0.0","lib 1.0.0","helper 1.0.0","internal 1.0.0"]}"#;
    let cmd = FakeCmd::new().with_metadata(meta);
    let clock = AdvancingClock::new();
    // Only `lib` is ever reported visible; if the adapter waited on internal/helper
    // it would time out and this test would fail rather than publish `bin`.
    let reg = DelayedRegistry::new("lib", "1.0.0", 1);
    let root = Path::new("/repo");
    let c = ctx_advancing(&cmd, &clock, &reg, root);

    let t = target_named(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "bin",
        "1.0.0",
    );
    resolve(Adapter::CargoPublish).publish(&c, &t).unwrap();

    // Only `bin` publishes — its dependency `lib` is another target's crate, and
    // `internal`/`helper` are not crates.io deps at all.
    assert_eq!(
        cmd.calls(),
        vec![
            "cargo metadata --no-deps --format-version 1",
            "cargo publish --registry crates-io -p bin",
        ]
    );
}

#[test]
fn empty_cargo_metadata_output_is_a_hard_error() {
    // A successful `cargo metadata` with no output must not silently degrade to a
    // single-crate publish — it is a broken host/runner and must fail loudly.
    let cmd = FakeCmd::new(); // no metadata stubbed ⇒ empty stdout
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Rust,
        Registry::CratesIo,
        Adapter::CargoPublish,
        "1.0.0",
    );
    let err = resolve(Adapter::CargoPublish).publish(&c, &t).unwrap_err();
    match err {
        AdapterError::Command { stderr, .. } => {
            assert!(stderr.contains("no output"), "got: {stderr}");
        }
        other => panic!("expected Command error, got {other:?}"),
    }
    assert!(!cmd.calls().iter().any(|call| call.contains("publish")));
}

#[test]
fn ci_only_pypi_publish_is_unsupported_from_host() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let t = target(
        Ecosystem::Python,
        Registry::Pypi,
        Adapter::GhActionPypiPublish,
        "1.0.0",
    );
    let err = resolve(Adapter::GhActionPypiPublish)
        .publish(&c, &t)
        .unwrap_err();
    assert!(matches!(
        err,
        AdapterError::Unsupported {
            operation: "publish",
            ..
        }
    ));
    // It must not have fabricated a publish either.
    assert!(cmd.calls().is_empty());
}

#[test]
fn homebrew_and_binary_have_no_build_phase() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    for (registry, id) in [
        (Registry::Homebrew, Adapter::HomebrewTap),
        (Registry::GhReleases, Adapter::Manual),
    ] {
        let t = target(Ecosystem::Binary, registry, id, "1.0.0");
        let b = resolve(id).build(&c, &t).unwrap();
        assert!(b.artifacts.is_empty(), "{id:?} should have no artifacts");
    }
    assert!(cmd.calls().is_empty(), "a no-op build shelled out");
}

// ── Threaded artifacts reach the two distribution adapters' publish() ─────────

#[test]
fn binary_publish_uploads_the_threaded_asset_paths() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = ReleaseArtifacts {
        assets: vec![
            "dist/tool-1.0.0-x86_64.tar.gz".to_string(),
            "dist/tool-1.0.0-aarch64.tar.gz".to_string(),
        ],
        source_tarball: None,
        repo_slug: None,
        homebrew: None,
    };
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::GhReleases,
        Adapter::Manual,
        "1.0.0",
    );
    resolve(Adapter::Manual).publish(&c, &t).unwrap();

    // Flags precede the `--` terminator; the threaded asset paths follow it.
    assert_eq!(
        cmd.calls(),
        vec![
            "gh release upload v1.0.0 --clobber -- dist/tool-1.0.0-x86_64.tar.gz \
             dist/tool-1.0.0-aarch64.tar.gz"
        ]
    );
}

#[test]
fn binary_publish_records_the_release_url_from_the_threaded_slug() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(7);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = ReleaseArtifacts {
        assets: vec!["dist/tool-1.0.0-x86_64.tar.gz".to_string()],
        source_tarball: None,
        repo_slug: Some("o/r".to_string()),
        homebrew: None,
    };
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::GhReleases,
        Adapter::Manual,
        "1.0.0",
    );
    let r = resolve(Adapter::Manual).publish(&c, &t).unwrap();

    // The upload is pinned to the threaded slug with `--repo`, so it targets the
    // same repository the receipt records (not gh's ambient resolution).
    assert_eq!(
        cmd.calls(),
        vec!["gh release upload v1.0.0 --repo o/r --clobber -- dist/tool-1.0.0-x86_64.tar.gz"]
    );
    // The receipt records where the assets landed: the GitHub-Release page for the
    // tag, built from the threaded slug. GitHub Releases expose no single publish
    // digest, so `digest` is honestly `None`.
    assert_eq!(
        r.remote_url.as_deref(),
        Some("https://github.com/o/r/releases/tag/v1.0.0")
    );
    assert_eq!(r.digest, None);
    assert_eq!(r.timestamp, 7, "receipt stamps the injected clock");
}

#[test]
fn binary_publish_records_no_url_without_a_slug() {
    // No resolvable GitHub remote ⇒ no threaded slug ⇒ the receipt honestly
    // records no `remote_url` rather than fabricating one.
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = ReleaseArtifacts {
        assets: vec!["dist/tool-1.0.0-x86_64.tar.gz".to_string()],
        source_tarball: None,
        repo_slug: None,
        homebrew: None,
    };
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::GhReleases,
        Adapter::Manual,
        "1.0.0",
    );
    let r = resolve(Adapter::Manual).publish(&c, &t).unwrap();
    assert_eq!(r.remote_url, None);
    assert_eq!(r.digest, None);
}

#[test]
fn homebrew_publish_reads_the_threaded_tarball_url_and_sha256() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = ReleaseArtifacts {
        assets: vec![],
        source_tarball: Some(SourceTarball {
            url: "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz".to_string(),
            sha256: Some("deadbeef".to_string()),
        }),
        repo_slug: Some("o/r".to_string()),
        homebrew: None,
    };
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "1.0.0",
    );
    resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap();

    // The bump carries the threaded `--url`/`--sha256`, with the formula name
    // after the `--` option terminator.
    assert_eq!(
        cmd.calls(),
        vec![
            "brew bump-formula-pr --url \
             https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz --sha256 deadbeef -- tool"
        ]
    );
}

#[test]
fn homebrew_core_publish_omits_sha256_when_not_yet_computed() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    // The sha256 is threaded as `None` by this layer (the digest lands with the
    // finished body); the bump then carries `--url` but no `--sha256`.
    let artifacts = ReleaseArtifacts {
        assets: vec![],
        source_tarball: Some(SourceTarball {
            url: "https://github.com/o/r/archive/refs/tags/v2.0.0.tar.gz".to_string(),
            sha256: None,
        }),
        repo_slug: Some("o/r".to_string()),
        homebrew: None,
    };
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewCore,
        "2.0.0",
    );
    resolve(Adapter::HomebrewCore).publish(&c, &t).unwrap();

    assert_eq!(
        cmd.calls(),
        vec![
            "brew bump-formula-pr --no-fork --url \
             https://github.com/o/r/archive/refs/tags/v2.0.0.tar.gz -- tool"
        ]
    );
}

// ── Homebrew first-formula bootstrap: create vs. bump ────────────────────────

/// The threaded homebrew inputs for a configured tap.
fn homebrew_artifacts(
    tap: &str,
    url: &str,
    sha256: Option<&str>,
    license: Option<&str>,
) -> ReleaseArtifacts {
    ReleaseArtifacts {
        assets: vec![],
        source_tarball: Some(SourceTarball {
            url: url.to_string(),
            sha256: sha256.map(str::to_string),
        }),
        repo_slug: Some("o/r".to_string()),
        homebrew: Some(HomebrewFormula {
            tap: Some(tap.to_string()),
            license: license.map(str::to_string),
        }),
    }
}

/// Locate the create path's private scratch checkout (now uniquely named
/// `ossctl-homebrew-<name>-<version>-<pid>-<nanos>`) by scanning `temp_dir` for
/// the `<name>-<version>` prefix, and read back the formula it wrote. Also
/// returns the dir so the caller can clean it up.
fn read_created_formula(name: &str, version: &str) -> (std::path::PathBuf, String) {
    let prefix = format!("ossctl-homebrew-{name}-{version}-");
    let dir = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
        .expect("the create path made a scratch checkout");
    let formula = std::fs::read_to_string(dir.join(format!("Formula/{name}.rb")))
        .expect("the create path wrote the formula file");
    (dir, formula)
}

#[test]
fn homebrew_tap_direct_writes_when_the_formula_already_exists() {
    // The tap already serves `Formula/tool.rb` (gh api probe exits 0) → the
    // tap-write path runs: clone → overwrite the formula → (dirty) commit → push
    // directly to the tap's default branch. NO `brew bump-formula-pr`, no PR.
    let cmd = FakeCmd::new()
        // The overwrite makes the checkout dirty, so the commit/push run.
        .stdout_calls_containing("status --porcelain", " M Formula/tool.rb\n");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz",
        Some("deadbeef"),
        Some("MIT"),
    );
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "1.0.0",
    );
    let r = resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap();

    let calls = cmd.calls();
    assert_eq!(
        calls[0],
        "gh api --silent repos/o/homebrew-r/contents/Formula/tool.rb"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("gh repo clone o/homebrew-r ")),
        "expected a tap clone: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("status --porcelain")),
        "expected a dirty-check: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("add Formula/tool.rb")),
        "expected the updated formula to be staged: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("commit -m tool 1.0.0")),
        "expected a commit: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.ends_with("push origin HEAD")),
        "expected a direct push to the default branch: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("bump-formula-pr")),
        "the tap-write path must not call bump-formula-pr: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("pr create")),
        "the tap-write path must not open a PR: {calls:?}"
    );
    // The receipt records the verified digest the formula pins.
    assert_eq!(r.digest.as_deref(), Some("deadbeef"));

    // The overwritten formula on disk carries the threaded url + verified sha256.
    let (workdir, formula) = read_created_formula("tool", "1.0.0");
    assert!(
        formula.contains("url \"https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz\""),
        "{formula}"
    );
    assert!(formula.contains("sha256 \"deadbeef\""), "{formula}");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn homebrew_tap_direct_write_is_a_noop_when_the_formula_is_unchanged() {
    // A resume/re-run at the target version: the overwrite produces byte-identical
    // content, so `git status --porcelain` is clean → the path is a no-op success
    // (clone + dirty-check only, no empty commit, no redundant push). The default
    // FakeCmd serves an empty (clean) `git status`.
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz",
        Some("deadbeef"),
        Some("MIT"),
    );
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "1.0.0",
    );
    let r = resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap();

    let calls = cmd.calls();
    assert!(
        calls.iter().any(|c| c.contains("status --porcelain")),
        "expected a dirty-check: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("commit")),
        "an unchanged formula must not create an empty commit: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.ends_with("push origin HEAD")),
        "an unchanged formula must not push: {calls:?}"
    );
    // Still a success receipt recording the verified digest.
    assert_eq!(r.digest.as_deref(), Some("deadbeef"));

    let (workdir, _) = read_created_formula("tool", "1.0.0");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn homebrew_tap_direct_write_fails_closed_without_a_verified_sha256() {
    // Fail-closed: the tap-write path pushes to the default branch (what
    // `brew install` resolves), so it REFUSES to write a formula without a verified
    // sha256 — no TODO placeholder, no broken install. The formula exists (probe
    // exits 0) so the path is TapWrite; sha256 is threaded as None.
    let cmd = FakeCmd::new().stdout_calls_containing("status --porcelain", " M Formula/tool.rb\n");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz",
        None,
        Some("MIT"),
    );
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "1.0.0",
    );
    let err = resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap_err();
    assert!(
        matches!(&err, AdapterError::Command { stderr, .. } if stderr.contains("verified sha256")),
        "expected a fail-closed error naming the missing verified sha256: {err:?}"
    );
    // It must not have committed or pushed a formula.
    let calls = cmd.calls();
    assert!(
        !calls
            .iter()
            .any(|c| c.contains("commit") || c.ends_with("push origin HEAD")),
        "must not commit/push without a verified sha256: {calls:?}"
    );
}

#[test]
fn homebrew_tap_creates_the_initial_formula_when_absent() {
    // A fresh tap has no `Formula/hbcreate.rb` (the gh api probe 404s) → the create
    // path clones the tap, writes the generated formula, commits it on a branch,
    // and opens a PR. The generated `.rb` carries url + sha256 + license + a cargo
    // source-build install stanza.
    let cmd = FakeCmd::new().fail_calls_containing("contents/", 1, "HTTP 404: Not Found");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v1.2.3.tar.gz",
        Some("cafef00d"),
        Some("MIT"),
    );
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target_named(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "hbcreate",
        "1.2.3",
    );
    resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap();

    // The probe, then the git/gh create sequence (no `bump-formula-pr`).
    let calls = cmd.calls();
    assert_eq!(
        calls[0],
        "gh api --silent repos/o/homebrew-r/contents/Formula/hbcreate.rb"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("gh repo clone o/homebrew-r")),
        "expected a tap clone: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("checkout -b ossctl-homebrew-hbcreate-1.2.3")),
        "expected a create branch: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("add Formula/hbcreate.rb")),
        "expected the new formula to be staged: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("push --set-upstream origin ossctl-homebrew-hbcreate-1.2.3")),
        "expected the branch to be pushed: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("gh pr create --repo o/homebrew-r")),
        "expected a PR to be opened: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("bump-formula-pr")),
        "the create path must not bump: {calls:?}"
    );

    // The generated formula file carries the url, sha256, license, and cargo build.
    let (workdir, formula) = read_created_formula("hbcreate", "1.2.3");
    assert!(formula.contains("class Hbcreate < Formula"), "{formula}");
    assert!(
        formula.contains("url \"https://github.com/o/r/archive/refs/tags/v1.2.3.tar.gz\""),
        "{formula}"
    );
    assert!(formula.contains("sha256 \"cafef00d\""), "{formula}");
    assert!(formula.contains("license \"MIT\""), "{formula}");
    assert!(
        formula.contains("homepage \"https://github.com/o/r\""),
        "{formula}"
    );
    assert!(
        formula.contains("system \"cargo\", \"install\""),
        "{formula}"
    );
    // sha256 present ⇒ a ready (non-draft) PR.
    assert!(
        calls
            .iter()
            .any(|c| c.contains("gh pr create") && !c.contains("--draft")),
        "sha256 present should open a ready PR: {calls:?}"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn homebrew_create_records_the_pr_url_as_remote_url() {
    let cmd = FakeCmd::new()
        .fail_calls_containing("contents/", 1, "HTTP 404")
        .stdout_calls_containing(
            "pr create",
            "Warning: 1 uncommitted change\nhttps://github.com/o/homebrew-r/pull/7\n",
        );
    let clock = FakeClock(9);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v4.5.6.tar.gz",
        Some("beefcafe"),
        Some("MIT"),
    );
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target_named(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "hburl",
        "4.5.6",
    );
    let r = resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap();

    // The receipt records the PR URL gh printed — the last https line, not the
    // preceding warning (the field already existed — no shape change) — and stamps
    // the injected clock.
    assert_eq!(
        r.remote_url.as_deref(),
        Some("https://github.com/o/homebrew-r/pull/7")
    );
    assert_eq!(r.timestamp, 9);

    let (workdir, _) = read_created_formula("hburl", "4.5.6");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn homebrew_create_generates_a_sha256_placeholder_when_absent() {
    // The coordinator threads sha256=None pre-tag; the generated formula then emits
    // a TODO placeholder (a valid source template the maintainer completes) rather
    // than a wrong digest, and the PR is opened as a draft with a blocker note.
    let cmd = FakeCmd::new().fail_calls_containing("contents/", 1, "404");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz",
        None,
        None,
    );
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target_named(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "hbnosha",
        "1.0.0",
    );
    resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap();

    let (workdir, formula) = read_created_formula("hbnosha", "1.0.0");
    assert!(formula.contains("# TODO: sha256"), "{formula}");
    // No license threaded ⇒ the stanza is omitted, not emitted empty.
    assert!(!formula.contains("license \""), "{formula}");
    // No sha256 ⇒ the PR is opened as a draft (not silently mergeable-but-broken).
    assert!(
        cmd.calls()
            .iter()
            .any(|c| c.contains("gh pr create") && c.contains("--draft")),
        "absent sha256 should open a draft PR: {:?}",
        cmd.calls()
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn homebrew_dry_run_reports_the_chosen_path() {
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz",
        Some("deadbeef"),
        Some("MIT"),
    );

    // Formula absent ⇒ dry-run previews the create path.
    let create_cmd = FakeCmd::new().fail_calls_containing("contents/", 1, "404");
    let cc = ctx_with(&create_cmd, &clock, &reg, root, &artifacts);
    let t = target(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "1.0.0",
    );
    let create = resolve(Adapter::HomebrewTap).dry_run(&cc, &t).unwrap();
    assert!(
        create.notes.iter().any(|n| n.contains("create path")),
        "{:?}",
        create.notes
    );
    assert!(
        create
            .planned_commands
            .iter()
            .any(|c| c.rendered().contains("gh pr create")),
        "create dry-run should preview the PR: {:?}",
        create.planned_commands
    );

    // Formula present ⇒ dry-run previews the tap-write path (clone → add → commit →
    // push directly to the tap's default branch — no bump-formula-pr, no PR).
    let write_cmd = FakeCmd::new();
    let wc = ctx_with(&write_cmd, &clock, &reg, root, &artifacts);
    let write = resolve(Adapter::HomebrewTap).dry_run(&wc, &t).unwrap();
    assert!(
        write.notes.iter().any(|n| n.contains("tap-write path")),
        "{:?}",
        write.notes
    );
    let rendered: Vec<String> = write
        .planned_commands
        .iter()
        .map(crate::protocol::release::PlannedCommand::rendered)
        .collect();
    assert!(
        rendered
            .iter()
            .any(|c| c.starts_with("gh repo clone o/homebrew-r ")),
        "{rendered:?}"
    );
    assert!(
        rendered.iter().any(|c| c.ends_with("push origin HEAD")),
        "{rendered:?}"
    );
    assert!(
        !rendered
            .iter()
            .any(|c| c.contains("bump-formula-pr") || c.contains("pr create")),
        "{rendered:?}"
    );
}

#[test]
fn homebrew_dry_run_previews_bump_pr_without_a_configured_tap() {
    // No configured tap (homebrew: None) ⇒ the bump-PR fallback for homebrew-core.
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = ReleaseArtifacts {
        assets: vec![],
        source_tarball: Some(SourceTarball {
            url: "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz".to_string(),
            sha256: Some("deadbeef".to_string()),
        }),
        repo_slug: Some("o/r".to_string()),
        homebrew: None,
    };
    let cmd = FakeCmd::new();
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);
    let t = target(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewCore,
        "1.0.0",
    );
    let bump = resolve(Adapter::HomebrewCore).dry_run(&c, &t).unwrap();
    assert!(
        bump.notes.iter().any(|n| n.contains("bump-PR path")),
        "{:?}",
        bump.notes
    );
    assert_eq!(bump.planned_commands.len(), 1);
    assert!(bump.planned_commands[0]
        .rendered()
        .starts_with("brew bump-formula-pr"));
    // No configured tap ⇒ no probe was needed.
    assert!(cmd.calls().is_empty(), "{:?}", cmd.calls());
}

#[test]
fn homebrew_probe_error_is_not_treated_as_absent() {
    // A non-404 probe failure (auth/rate-limit/network) must abort — NOT be read as
    // "absent" and trigger a create that could overwrite an existing formula.
    let cmd = FakeCmd::new().fail_calls_containing("contents/", 1, "HTTP 403: rate limit exceeded");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz",
        Some("deadbeef"),
        Some("MIT"),
    );
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "1.0.0",
    );
    let err = resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap_err();
    assert!(matches!(err, AdapterError::Command { .. }), "got {err:?}");
    // Only the probe ran — no clone, no bump, no create.
    assert_eq!(
        cmd.calls().len(),
        1,
        "must abort after the probe: {:?}",
        cmd.calls()
    );
}

#[test]
fn homebrew_create_class_name_handles_a_leading_digit() {
    // A Ruby constant cannot start with a digit; the class name is prefixed with X.
    let cmd = FakeCmd::new().fail_calls_containing("contents/", 1, "404");
    let clock = FakeClock(1);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let artifacts = homebrew_artifacts(
        "o/homebrew-r",
        "https://github.com/o/r/archive/refs/tags/v1.0.0.tar.gz",
        Some("deadbeef"),
        None,
    );
    let c = ctx_with(&cmd, &clock, &reg, root, &artifacts);

    let t = target_named(
        Ecosystem::Binary,
        Registry::Homebrew,
        Adapter::HomebrewTap,
        "3d-tool",
        "1.0.0",
    );
    resolve(Adapter::HomebrewTap).publish(&c, &t).unwrap();

    let (workdir, formula) = read_created_formula("3d-tool", "1.0.0");
    assert!(formula.contains("class X3dTool < Formula"), "{formula}");
    let _ = std::fs::remove_dir_all(&workdir);
}

// ── classify_receipt: the pure verify core, all four outcomes ────────────────

#[test]
fn classify_unknown_when_lookup_absent() {
    let r = receipt(Ecosystem::Rust, "1.0.0", None);
    assert_eq!(classify_receipt(&r, None), VerifyOutcome::Unknown);
}

#[test]
fn classify_missing_when_version_absent() {
    let r = receipt(Ecosystem::Rust, "1.0.0", None);
    let obs = RemoteObservation {
        published_versions: vec!["0.9.0".to_string()],
        remote_digest: None,
    };
    assert_eq!(classify_receipt(&r, Some(&obs)), VerifyOutcome::Missing);
}

#[test]
fn classify_matches_when_version_present() {
    let r = receipt(Ecosystem::Rust, "1.0.0", None);
    let obs = RemoteObservation {
        published_versions: vec!["0.9.0".to_string(), "1.0.0".to_string()],
        remote_digest: None,
    };
    assert_eq!(classify_receipt(&r, Some(&obs)), VerifyOutcome::Matches);
}

#[test]
fn classify_matches_when_digests_agree() {
    let r = receipt(Ecosystem::Rust, "1.0.0", Some("sha256:aaa"));
    let obs = RemoteObservation {
        published_versions: vec!["1.0.0".to_string()],
        remote_digest: Some("sha256:aaa".to_string()),
    };
    assert_eq!(classify_receipt(&r, Some(&obs)), VerifyOutcome::Matches);
}

#[test]
fn classify_conflicts_on_digest_mismatch() {
    let r = receipt(Ecosystem::Rust, "1.0.0", Some("sha256:aaa"));
    let obs = RemoteObservation {
        published_versions: vec!["1.0.0".to_string()],
        remote_digest: Some("sha256:bbb".to_string()),
    };
    assert_eq!(classify_receipt(&r, Some(&obs)), VerifyOutcome::Conflicts);
}

// ── verify() through the default RegistryQuery path ──────────────────────────

#[test]
fn verify_matches_via_registry() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]);
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let r = receipt(Ecosystem::Rust, "1.0.0", None);
    let out = resolve(Adapter::CargoPublish).verify(&c, &r).unwrap();
    assert_eq!(out, VerifyOutcome::Matches);
}

#[test]
fn verify_missing_via_registry() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new().with("rust", "tool", &["0.1.0"]);
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let r = receipt(Ecosystem::Rust, "1.0.0", None);
    let out = resolve(Adapter::CargoPublish).verify(&c, &r).unwrap();
    assert_eq!(out, VerifyOutcome::Missing);
}

#[test]
fn verify_unknown_on_registry_outage() {
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::erroring();
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let r = receipt(Ecosystem::Rust, "1.0.0", None);
    // A registry outage must never be read as "did not land".
    let out = resolve(Adapter::CargoPublish).verify(&c, &r).unwrap();
    assert_eq!(out, VerifyOutcome::Unknown);
}

#[test]
fn homebrew_and_binary_verify_is_always_unknown() {
    // Even with a registry that *would* answer, these adapters report Unknown
    // because their destination is not observable through RegistryQuery.
    let cmd = FakeCmd::new();
    let clock = FakeClock(1);
    let reg = FakeRegistry::new().with("binary", "tool", &["1.0.0"]);
    let root = Path::new("/repo");
    let c = ctx(&cmd, &clock, &reg, root);

    let r = receipt(Ecosystem::Binary, "1.0.0", None);
    for id in [Adapter::HomebrewTap, Adapter::HomebrewCore, Adapter::Manual] {
        assert_eq!(
            resolve(id).verify(&c, &r).unwrap(),
            VerifyOutcome::Unknown,
            "{id:?} must report Unknown"
        );
    }
}

// ── Wire parity: as_str() must equal the serde form (they must never drift) ──

#[test]
fn verify_outcome_as_str_matches_serde() {
    for v in [
        VerifyOutcome::Matches,
        VerifyOutcome::Conflicts,
        VerifyOutcome::Missing,
        VerifyOutcome::Unknown,
    ] {
        assert_eq!(
            serde_json::to_value(v).unwrap(),
            serde_json::Value::String(v.as_str().to_string()),
            "as_str() drifted from serde for {v:?}"
        );
    }
}

// ── CI-delegation capability (release-engine-cut-cargo-dist-flow) ─────────────

/// The CI-delegated set is exactly the three adapters whose publish is produced by
/// the tag-triggered CI, and each of them returns `Unsupported` from `publish`.
///
/// This is the invariant the coordinator relies on: it skips a target **iff** the
/// adapter declares [`is_ci_delegated`](ReleaseAdapter::is_ci_delegated), never by
/// catching an `Unsupported`. Because every `Unsupported`-returning adapter *is*
/// delegated (proven here), a non-delegated adapter can never return `Unsupported`
/// for the coordinator to swallow — a genuine `Unsupported` would reach the
/// coordinator's `Err => fail_phase` path and still fail the cut.
#[test]
fn ci_delegation_matches_the_unsupported_publishers() {
    let delegated = [
        Adapter::CargoDist,
        Adapter::ReleasePlease,
        Adapter::GhActionPypiPublish,
    ];
    let all = [
        Adapter::CargoPublish,
        Adapter::CargoDist,
        Adapter::ReleasePlease,
        Adapter::Changesets,
        Adapter::NpmPublish,
        Adapter::GhActionPypiPublish,
        Adapter::Twine,
        Adapter::Goreleaser,
        Adapter::HomebrewTap,
        Adapter::HomebrewCore,
        Adapter::Manual,
    ];
    // 1. The capability flag is set for exactly the delegated set.
    for id in all {
        assert_eq!(
            resolve(id).is_ci_delegated(),
            delegated.contains(&id),
            "is_ci_delegated() is wrong for {id:?}"
        );
    }
    // 2. Each delegated adapter's publish is an honest `Unsupported` (returned
    //    before any command, so no fakes are exercised).
    let runner = FakeCmd::new();
    let clock = FakeClock(0);
    let reg = FakeRegistry::new();
    let root = Path::new("/repo");
    let c = ctx(&runner, &clock, &reg, root);
    for id in delegated {
        let t = target(Ecosystem::Rust, Registry::GhReleases, id, "1.0.0");
        assert!(
            matches!(
                resolve(id).publish(&c, &t),
                Err(AdapterError::Unsupported {
                    operation: "publish",
                    ..
                })
            ),
            "delegated adapter {id:?} must publish() -> Unsupported"
        );
    }
}

/// `ci_owns_github_release()` is a **strict subset** of `is_ci_delegated()`: only
/// `cargo-dist` (whose `release.yml` runs `gh release create`) owns the shared
/// GitHub Release. The regression this guards
/// (`coordinator-release-vs-cargo-dist-ownership`): a CI-delegated *publish* that
/// does NOT touch the GitHub Release — `gh-action-pypi-publish` (uploads to `PyPI`) and
/// `release-please` (publish-on-merge) — must NOT suppress the coordinator's Release
/// creation, or a pure-Python trusted-publisher plan would silently lose its
/// engine-created GitHub Release.
#[test]
fn only_cargo_dist_owns_the_github_release_and_it_is_a_subset_of_ci_delegation() {
    let all = [
        Adapter::CargoPublish,
        Adapter::CargoDist,
        Adapter::ReleasePlease,
        Adapter::Changesets,
        Adapter::NpmPublish,
        Adapter::GhActionPypiPublish,
        Adapter::Twine,
        Adapter::Goreleaser,
        Adapter::HomebrewTap,
        Adapter::HomebrewCore,
        Adapter::Manual,
    ];
    for id in all {
        let owns = resolve(id).ci_owns_github_release();
        assert_eq!(
            owns,
            id == Adapter::CargoDist,
            "ci_owns_github_release() is wrong for {id:?}"
        );
        // Subset invariant: owning the Release implies CI-delegation.
        if owns {
            assert!(
                resolve(id).is_ci_delegated(),
                "{id:?} owns the Release but is not CI-delegated"
            );
        }
    }
    // The two delegated-but-not-Release-owning adapters explicitly do NOT own it.
    assert!(!resolve(Adapter::GhActionPypiPublish).ci_owns_github_release());
    assert!(!resolve(Adapter::ReleasePlease).ci_owns_github_release());
}
