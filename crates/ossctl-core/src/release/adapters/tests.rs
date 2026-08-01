//! Unit tests for the release adapter layer: registry dispatch to each
//! ecosystem, the `dry_run` command plans, `build`/`publish` driving the
//! injected runner, and `verify` classification across all four outcomes —
//! including the outage ⇒ `Unknown` path (never a false `Missing`).

use std::cell::RefCell;
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
}

impl FakeCmd {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail: None,
            spawn_err: false,
        }
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
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl CommandRunner for FakeCmd {
    fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
        self.calls
            .borrow_mut()
            .push(format!("{program} {}", args.join(" ")));
        if self.spawn_err {
            return Err(io::Error::from(io::ErrorKind::NotFound));
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

/// Programmable registry: maps `(ecosystem, package)` to a version list, or errors.
struct FakeRegistry {
    versions: HashMap<(String, String), Vec<String>>,
    err: bool,
}
impl FakeRegistry {
    fn new() -> Self {
        Self {
            versions: HashMap::new(),
            err: false,
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
}
impl RegistryQuery for FakeRegistry {
    fn published_versions(&self, ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
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

fn target(
    ecosystem: Ecosystem,
    registry: Registry,
    adapter: Adapter,
    version: &str,
) -> AdapterTarget {
    AdapterTarget {
        target: Target {
            ecosystem,
            package: Some("tool".to_string()),
            registry,
            adapter,
        },
        package: "tool".to_string(),
        version: version.to_string(),
    }
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
fn dry_run_plans_commands_without_running_them() {
    let cmd = FakeCmd::new();
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
    assert_eq!(report.planned_commands.len(), 1);
    assert_eq!(
        report.planned_commands[0].rendered(),
        "cargo publish -p tool --dry-run"
    );
    // dry_run must not execute anything.
    assert!(cmd.calls().is_empty(), "dry_run ran a command");
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
    let cmd = FakeCmd::new();
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

    assert_eq!(cmd.calls(), vec!["cargo publish -p tool"]);
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
