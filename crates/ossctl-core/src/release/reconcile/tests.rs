//! Unit tests for the read-only reconcile engine: every [`VerifyOutcome`] class
//! (including `Unknown`-on-lookup-failure and the structurally-unobservable
//! distribution targets), the roll-up summary, stable ordering, and a proof that
//! reconciling performs **no** command execution (read-only, registry query only).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::Path;

use super::{reconcile, reconcile_with_plan};
use crate::contract::schema::{Adapter, Ecosystem, Registry};
use crate::ports::{Clock, CommandOutput, CommandRunner, RegistryQuery};
use crate::protocol::journal::{PublishReceipt, RunState, RunStatus};
use crate::protocol::plan::{PlanTarget, ReleasePlan};
use crate::protocol::release::VerifyOutcome;
use crate::release::adapters::EffectCtx;

// ── Fakes ────────────────────────────────────────────────────────────────────

/// Records every read-only observation command.
#[derive(Default)]
struct RecordingCmd {
    calls: RefCell<Vec<String>>,
}
impl CommandRunner for RecordingCmd {
    fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
        self.calls
            .borrow_mut()
            .push(format!("{program} {}", args.join(" ")));
        let stdout = if program == "git" && args.starts_with(&["rev-list"]) {
            "abc123\n".to_string()
        } else if program == "git" && args.starts_with(&["grep"]) {
            ".github/workflows/publish-crates.yml\n".to_string()
        } else if program == "gh" && args.starts_with(&["run", "list"]) {
            let branch = args
                .iter()
                .position(|arg| *arg == "--branch")
                .and_then(|index| args.get(index + 1))
                .copied()
                .unwrap_or("v1.0.0");
            format!(
                r#"[{{"databaseId":42,"status":"completed","conclusion":"success","headBranch":"{branch}","headSha":"abc123","url":"https://github.com/acme/tool/actions/runs/42"}}]"#
            )
        } else if program == "gh" && args.starts_with(&["release", "download"]) {
            r#"{"announcement_tag":"v1.0.0","releases":[{"app_name":"project-canon-cli","app_version":"1.0.0","artifacts":["tool-aarch64-apple-darwin.tar.xz","tool-aarch64-unknown-linux-musl.tar.xz","tool-x86_64-unknown-linux-musl.tar.xz","tool-installer.sh"]}]}"#.to_string()
        } else if program == "gh" {
            r#"{"tagName":"v1.0.0","name":"1.0.0 - 2026-08-17","isDraft":false,"assets":[{"name":"dist-manifest.json"},{"name":"tool-aarch64-apple-darwin.tar.xz"},{"name":"tool-aarch64-unknown-linux-musl.tar.xz"},{"name":"tool-x86_64-unknown-linux-musl.tar.xz"},{"name":"tool-installer.sh"}]}"#.to_string()
        } else {
            String::new()
        };
        Ok(CommandOutput {
            status: Some(0),
            stdout,
            stderr: String::new(),
        })
    }
}

struct CargoDistCmd {
    view: String,
    manifest: String,
}
impl CommandRunner for CargoDistCmd {
    fn run(&self, _program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
        Ok(CommandOutput {
            status: Some(0),
            stdout: if args.starts_with(&["release", "download"]) {
                self.manifest.clone()
            } else {
                self.view.clone()
            },
            stderr: String::new(),
        })
    }
}

struct FormulaRegistry {
    status: u16,
    body: Vec<u8>,
    urls: RefCell<Vec<String>>,
}
impl FormulaRegistry {
    fn cargo_dist_fixture() -> Self {
        Self {
            status: 200,
            body: include_bytes!("../fixtures/project-canon-cargo-dist-0.28.2.rb").to_vec(),
            urls: RefCell::new(Vec::new()),
        }
    }

    fn with_status(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
            urls: RefCell::new(Vec::new()),
        }
    }
}
impl RegistryQuery for FormulaRegistry {
    fn http_get(&self, url: &str) -> io::Result<(u16, Vec<u8>)> {
        self.urls.borrow_mut().push(url.to_string());
        Ok((self.status, self.body.clone()))
    }

    fn published_versions(&self, _ecosystem: &str, _package: &str) -> io::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn now_unix(&self) -> u64 {
        0
    }
}

/// Programmable registry: `(ecosystem, package) -> versions`, or a global outage.
struct FakeRegistry {
    versions: HashMap<(String, String), Vec<String>>,
    outage: bool,
    queried: RefCell<Vec<(String, String)>>,
}
impl FakeRegistry {
    fn new() -> Self {
        Self {
            versions: HashMap::new(),
            outage: false,
            queried: RefCell::new(Vec::new()),
        }
    }
    fn with(mut self, ecosystem: &str, package: &str, versions: &[&str]) -> Self {
        self.versions.insert(
            (ecosystem.to_string(), package.to_string()),
            versions.iter().map(|s| (*s).to_string()).collect(),
        );
        self
    }
    fn outage() -> Self {
        Self {
            outage: true,
            ..Self::new()
        }
    }
}
impl RegistryQuery for FakeRegistry {
    fn http_get(&self, _url: &str) -> io::Result<(u16, Vec<u8>)> {
        if self.outage {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        Ok((
            200,
            b"# Generated by ossctl; do not edit by hand (template-version: 2)\n\
              class Tool < Formula\n  version \"1.0.0\"\n  if OS.mac? && Hardware::CPU.arm?\n    url \"https://example/tool.tar.xz\"\n    sha256 \"deadbeef\"\n  end\nend\n"
                .to_vec(),
        ))
    }

    fn published_versions(&self, ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        self.queried
            .borrow_mut()
            .push((ecosystem.to_string(), package.to_string()));
        if self.outage {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        Ok(self
            .versions
            .get(&(ecosystem.to_string(), package.to_string()))
            .cloned()
            .unwrap_or_default())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn receipt(
    ecosystem: &str,
    package: Option<&str>,
    version: &str,
    digest: Option<&str>,
) -> PublishReceipt {
    PublishReceipt {
        ecosystem: ecosystem.to_string(),
        package: package.map(str::to_string),
        version: version.to_string(),
        registry_url: None,
        digest: digest.map(str::to_string),
    }
}

/// A `RunState` with the given `(target_id, receipt)` published set.
fn state_with(published: &[(&str, PublishReceipt)]) -> RunState {
    let mut s = RunState::empty();
    s.run_id = "RUN01".into();
    s.plan_id = "plan-abc".into();
    s.status = RunStatus::Completed;
    s.applied_seq = 7;
    s.targets = published.iter().map(|(id, _)| (*id).to_string()).collect();
    for (id, r) in published {
        s.published.insert((*id).to_string(), r.clone());
    }
    s
}

fn run<'a>(
    state: &RunState,
    cmd: &'a RecordingCmd,
    clock: &'a FixedClock,
    reg: &'a FakeRegistry,
) -> crate::protocol::reconcile::ReconcileReport {
    let ctx = EffectCtx {
        runner: cmd,
        clock,
        registry: reg,
        repo_root: Path::new("/repo"),
        artifacts: &crate::release::adapters::EMPTY_ARTIFACTS,
    };
    reconcile(state, &ctx)
}

fn only(
    report: &crate::protocol::reconcile::ReconcileReport,
) -> &crate::protocol::reconcile::TargetReconcile {
    assert_eq!(report.targets.len(), 1, "expected exactly one target");
    &report.targets[0]
}

// ── The four outcomes ────────────────────────────────────────────────────────

#[test]
fn matches_when_version_is_published() {
    let state = state_with(&[("cargo", receipt("rust", Some("tool"), "1.0.0", None))]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["0.9.0", "1.0.0"]);
    let report = run(&state, &cmd, &clock, &reg);

    let t = only(&report);
    assert_eq!(t.outcome, VerifyOutcome::Matches);
    assert_eq!(t.detail, None, "a clean match carries no detail");
    assert_eq!(report.summary.matches, 1);
    assert_eq!(report.summary.reconciled, 1);
    // Registry was consulted with the receipt's own coordinates.
    assert_eq!(
        reg.queried.borrow().as_slice(),
        &[("rust".to_string(), "tool".to_string())]
    );
}

#[test]
fn missing_when_version_absent_from_registry() {
    let state = state_with(&[("cargo", receipt("rust", Some("tool"), "1.0.0", None))]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["0.9.0"]);
    let report = run(&state, &cmd, &clock, &reg);

    let t = only(&report);
    assert_eq!(t.outcome, VerifyOutcome::Missing);
    assert!(t.detail.as_deref().unwrap().contains("does not report"));
    assert_eq!(report.summary.missing, 1);
}

#[test]
fn a_recorded_digest_does_not_force_conflicts_without_a_remote_digest() {
    // `RegistryQuery` lists versions but exposes no remote digest, so a present
    // version resolves to Matches even when the receipt carries a local digest —
    // presence, not a false Conflicts (the digest-mismatch path is covered by the
    // adapter layer's own `classify_receipt` tests, where a remote digest exists).
    let state = state_with(&[(
        "npm",
        receipt("node", Some("@scope/tool"), "2.0.0", Some("sha256:local")),
    )]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("node", "@scope/tool", &["2.0.0"]);
    let report = run(&state, &cmd, &clock, &reg);
    assert_eq!(only(&report).outcome, VerifyOutcome::Matches);
}

#[test]
fn unknown_on_registry_outage_never_missing() {
    let state = state_with(&[("cargo", receipt("rust", Some("tool"), "1.0.0", None))]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::outage();
    let report = run(&state, &cmd, &clock, &reg);

    let t = only(&report);
    assert_eq!(
        t.outcome,
        VerifyOutcome::Unknown,
        "an outage must never read as Missing"
    );
    assert!(t
        .detail
        .as_deref()
        .unwrap()
        .contains("could not be performed"));
    assert_eq!(report.summary.unknown, 1);
    assert_eq!(report.summary.missing, 0);
}

#[test]
fn distribution_targets_are_observable_and_fetch_failure_is_unknown_not_green() {
    let gh = state_with(&[("gh", receipt("binary", Some("tool"), "1.0.0", None))]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new();
    let report = run(&gh, &cmd, &clock, &reg);
    assert_eq!(only(&report).outcome, VerifyOutcome::Matches);
    assert_eq!(
        cmd.calls.borrow().as_slice(),
        &["gh release view v1.0.0 --json assets,isDraft,tagName"]
    );

    let mut formula = receipt("binary", Some("tool"), "1.0.0", None);
    formula.registry_url =
        Some("https://github.com/acme/homebrew-tools/blob/HEAD/Formula/tool.rb".to_string());
    let tap = state_with(&[("homebrew", formula)]);
    let formula_cmd = RecordingCmd::default();
    let report = run(&tap, &formula_cmd, &clock, &reg);
    assert_eq!(only(&report).outcome, VerifyOutcome::Matches);
    assert!(formula_cmd.calls.borrow().is_empty());

    let failed = RecordingCmd::default();
    let outage = FakeRegistry::outage();
    let report = run(&tap, &failed, &clock, &outage);
    assert_eq!(only(&report).outcome, VerifyOutcome::Unknown);
    assert_ne!(
        only(&report).outcome,
        VerifyOutcome::Matches,
        "an unobservable destination must fail the mandatory verify barrier"
    );
    assert!(failed.calls.borrow().is_empty());
    assert!(reg.queried.borrow().is_empty());
}

#[test]
fn delegated_cargo_dist_uses_the_tagged_release_not_registry_or_title() {
    // Regression for project-canon v0.4.0/v0.5.0: its contract listed four
    // installation platforms, while cargo-dist deliberately built three. The
    // Release was complete according to cargo-dist (including dist-manifest.json),
    // but reconstructing archive names from contract policy false-red'd it. Its
    // title also differs from the tag and the package differs from the project.
    let target = PlanTarget {
        ecosystem: Ecosystem::Rust,
        package: Some("project-canon-cli".to_string()),
        registry: Registry::GhReleases,
        adapter: Adapter::CargoDist,
    };
    let plan = ReleasePlan {
        plan_id: "plan-abc".to_string(),
        contract_schema_version: 2,
        head_sha: "abc123".to_string(),
        version: "1.0.0".to_string(),
        targets: vec![target],
        phases: Vec::new(),
        bump: None,
        homebrew_tap: None,
        license: None,
        description: None,
        homebrew_platforms: vec![
            "aarch64-apple-darwin".to_string(),
            "x86_64-apple-darwin".to_string(),
            "aarch64-unknown-linux-musl".to_string(),
            "x86_64-unknown-linux-musl".to_string(),
        ],
    };
    let mut state = state_with(&[]);
    state.targets = vec!["rust".to_string()];
    state.delegated.insert("rust".to_string());
    state
        .delegated_adapters
        .insert("rust".to_string(), "cargo-dist".to_string());
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new();
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: Path::new("/repo"),
        artifacts: &crate::release::adapters::EMPTY_ARTIFACTS,
    };

    let report = reconcile_with_plan(&state, Some(&plan), &ctx);

    assert_eq!(only(&report).outcome, VerifyOutcome::Matches);
    assert!(reg.queried.borrow().is_empty(), "must not query a registry");
    assert_eq!(
        cmd.calls.borrow().as_slice(),
        &[
            "git rev-list -n 1 v1.0.0",
            "gh run list --workflow release.yml --branch v1.0.0 --event push --json databaseId,status,conclusion,headBranch,headSha,url --limit 20",
            "gh release view v1.0.0 --json assets,isDraft,tagName",
            "gh release download v1.0.0 --pattern dist-manifest.json --output -",
        ]
    );
}

#[test]
fn delegated_cargo_dist_reconciles_github_and_homebrew_at_their_own_destinations() {
    let targets = vec![
        PlanTarget {
            ecosystem: Ecosystem::Binary,
            package: Some("project-canon-cli".to_string()),
            registry: Registry::GhReleases,
            adapter: Adapter::CargoDist,
        },
        PlanTarget {
            ecosystem: Ecosystem::Binary,
            package: Some("project-canon".to_string()),
            registry: Registry::Homebrew,
            adapter: Adapter::CargoDist,
        },
    ];
    let ids = crate::release::journal_target_ids(&targets);
    let plan = ReleasePlan {
        plan_id: "plan-abc".to_string(),
        contract_schema_version: 2,
        head_sha: "abc123".to_string(),
        version: "0.6.1".to_string(),
        targets,
        phases: Vec::new(),
        bump: None,
        homebrew_tap: Some("jarimustonen/homebrew-project-canon".to_string()),
        license: Some("MIT".to_string()),
        description: None,
        homebrew_platforms: vec![
            "aarch64-apple-darwin".to_string(),
            "aarch64-unknown-linux-musl".to_string(),
            "x86_64-unknown-linux-musl".to_string(),
        ],
    };
    let mut state = state_with(&[]);
    state.targets = ids.clone();
    for id in &ids {
        state.delegated.insert(id.clone());
        state
            .delegated_adapters
            .insert(id.clone(), "cargo-dist".to_string());
    }
    let cmd = CargoDistCmd {
        view: r#"{"tagName":"v0.6.1","isDraft":false,"assets":[{"name":"dist-manifest.json"},{"name":"project-canon-cli-aarch64-apple-darwin.tar.xz"}]}"#.to_string(),
        manifest: r#"{"announcement_tag":"v0.6.1","releases":[{"app_name":"project-canon-cli","app_version":"0.6.1","artifacts":["project-canon-cli-aarch64-apple-darwin.tar.xz"]}]}"#.to_string(),
    };
    let (clock, reg) = (FixedClock, FormulaRegistry::cargo_dist_fixture());
    let ctx = EffectCtx {
        runner: &cmd,
        clock: &clock,
        registry: &reg,
        repo_root: Path::new("/repo"),
        artifacts: &crate::release::adapters::EMPTY_ARTIFACTS,
    };

    let report = reconcile_with_plan(&state, Some(&plan), &ctx);

    assert_eq!(report.summary.matches, 2, "{report:?}");
    assert_eq!(report.summary.unknown, 0);
    assert!(report
        .targets
        .iter()
        .all(|target| target.outcome == VerifyOutcome::Matches));
    assert_eq!(reg.urls.borrow().len(), 1, "Homebrew must be observed once");
    assert!(reg.urls.borrow()[0]
        .contains("jarimustonen/homebrew-project-canon/HEAD/Formula/project-canon.rb"));
}

#[test]
fn delegated_homebrew_distinguishes_missing_from_observer_failure() {
    let target = PlanTarget {
        ecosystem: Ecosystem::Binary,
        package: Some("project-canon".to_string()),
        registry: Registry::Homebrew,
        adapter: Adapter::CargoDist,
    };
    let plan = ReleasePlan {
        plan_id: "plan-abc".to_string(),
        contract_schema_version: 2,
        head_sha: "abc123".to_string(),
        version: "0.6.1".to_string(),
        targets: vec![target],
        phases: Vec::new(),
        bump: None,
        homebrew_tap: Some("jarimustonen/homebrew-project-canon".to_string()),
        license: None,
        description: None,
        homebrew_platforms: vec!["aarch64-apple-darwin".to_string()],
    };
    let mut state = state_with(&[]);
    state.targets = vec!["binary".to_string()];
    state.delegated.insert("binary".to_string());
    state
        .delegated_adapters
        .insert("binary".to_string(), "cargo-dist".to_string());
    let cmd = RecordingCmd::default();
    let clock = FixedClock;

    for (status, expected) in [(404, VerifyOutcome::Missing), (503, VerifyOutcome::Unknown)] {
        let reg = FormulaRegistry::with_status(status);
        let ctx = EffectCtx {
            runner: &cmd,
            clock: &clock,
            registry: &reg,
            repo_root: Path::new("/repo"),
            artifacts: &crate::release::adapters::EMPTY_ARTIFACTS,
        };
        let report = reconcile_with_plan(&state, Some(&plan), &ctx);
        assert_eq!(only(&report).outcome, expected, "HTTP {status}");
    }
}

#[test]
fn cargo_dist_manifest_inventory_must_be_complete_and_parseable() {
    let view = r#"{"tagName":"v1.0.0","isDraft":false,"assets":[{"name":"dist-manifest.json"},{"name":"tool-aarch64-apple-darwin.tar.xz"}]}"#;
    let missing_asset_manifest = r#"{"announcement_tag":"v1.0.0","releases":[{"app_name":"project-canon-cli","app_version":"1.0.0","artifacts":["tool-aarch64-apple-darwin.tar.xz","tool-x86_64-unknown-linux-musl.tar.xz"]}]}"#;
    let clock = FixedClock;
    let reg = FakeRegistry::new();
    for (manifest, expected) in [
        (missing_asset_manifest, VerifyOutcome::Missing),
        (
            r#"{"announcement_tag":"v1.0.0","releases":[{"app_name":"project-canon-cli","app_version":"1.0.0","artifacts":[]}]}"#,
            VerifyOutcome::Missing,
        ),
        (
            r#"{"announcement_tag":"v1.0.0","releases":[{"app_name":"another-app","app_version":"1.0.0","artifacts":["tool-aarch64-apple-darwin.tar.xz"]}]}"#,
            VerifyOutcome::Unknown,
        ),
        (
            r#"{"announcement_tag":"v1.0.0","releases":[{"app_name":"project-canon-cli","app_version":"2.0.0","artifacts":["tool-aarch64-apple-darwin.tar.xz"]}]}"#,
            VerifyOutcome::Conflicts,
        ),
        (
            r#"{"announcement_tag":"v1.0.0","releases":"new schema"}"#,
            VerifyOutcome::Unknown,
        ),
        ("not json", VerifyOutcome::Unknown),
    ] {
        let cmd = CargoDistCmd {
            view: view.to_string(),
            manifest: manifest.to_string(),
        };
        let ctx = EffectCtx {
            runner: &cmd,
            clock: &clock,
            registry: &reg,
            repo_root: Path::new("/repo"),
            artifacts: &crate::release::adapters::EMPTY_ARTIFACTS,
        };
        assert_eq!(
            crate::release::adapters::observe_cargo_dist_github_release(
                &ctx,
                "1.0.0",
                "project-canon-cli"
            ),
            expected
        );
    }
}

#[test]
fn unknown_when_receipt_has_no_package_name() {
    let state = state_with(&[("cargo", receipt("rust", None, "1.0.0", None))]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]);
    let report = run(&state, &cmd, &clock, &reg);

    let t = only(&report);
    assert_eq!(t.outcome, VerifyOutcome::Unknown);
    assert!(t.detail.as_deref().unwrap().contains("no package name"));
    // No package ⇒ no query fabricated against the registry.
    assert!(reg.queried.borrow().is_empty());
}

#[test]
fn unknown_for_unrecognized_ecosystem() {
    let state = state_with(&[("weird", receipt("cocoapods", Some("Tool"), "1.0.0", None))]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new();
    let report = run(&state, &cmd, &clock, &reg);

    let t = only(&report);
    assert_eq!(t.outcome, VerifyOutcome::Unknown);
    assert!(t
        .detail
        .as_deref()
        .unwrap()
        .contains("unrecognized ecosystem"));
    assert!(reg.queried.borrow().is_empty());
}

// ── Roll-up, ordering, and read-only guarantees ──────────────────────────────

#[test]
fn summary_tallies_a_mixed_run_and_targets_are_sorted() {
    let state = state_with(&[
        ("npm", receipt("node", Some("tool"), "3.0.0", None)), // Missing
        ("cargo", receipt("rust", Some("tool"), "1.0.0", None)), // Matches
        ("gh", receipt("binary", Some("tool"), "1.0.0", None)), // Matches via GitHub
    ]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new()
        .with("rust", "tool", &["1.0.0"])
        .with("node", "tool", &["1.0.0"]); // 3.0.0 absent ⇒ Missing
    let report = run(&state, &cmd, &clock, &reg);

    // BTreeMap iteration ⇒ target ids sorted: cargo, gh, npm.
    let ids: Vec<&str> = report.targets.iter().map(|t| t.target.as_str()).collect();
    assert_eq!(ids, vec!["cargo", "gh", "npm"]);

    assert_eq!(report.summary.reconciled, 3);
    assert_eq!(report.summary.matches, 2);
    assert_eq!(report.summary.missing, 1);
    assert_eq!(report.summary.unknown, 0);
    assert_eq!(report.summary.conflicts, 0);

    // Report echoes run identity + status.
    assert_eq!(report.run_id, "RUN01");
    assert_eq!(report.plan_id, "plan-abc");
    assert_eq!(report.run_status, RunStatus::Completed);
}

#[test]
fn empty_published_set_reconciles_to_an_empty_report() {
    let state = state_with(&[]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new();
    let report = run(&state, &cmd, &clock, &reg);

    assert!(report.targets.is_empty());
    assert_eq!(
        report.summary,
        crate::protocol::reconcile::ReconcileSummary::default()
    );
}

#[test]
fn reconcile_runs_only_the_exact_read_only_observation_commands() {
    // Destination observations may shell out, but only through read-only queries.
    let state = state_with(&[
        ("cargo", receipt("rust", Some("tool"), "1.0.0", None)),
        ("gh", receipt("binary", Some("tool"), "1.0.0", None)),
    ]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]);
    let _ = run(&state, &cmd, &clock, &reg);

    assert_eq!(
        cmd.calls.borrow().as_slice(),
        &["gh release view v1.0.0 --json assets,isDraft,tagName"],
        "reconcile must never execute a mutating command"
    );
}

#[test]
fn report_serializes_under_the_envelope_shape() {
    let state = state_with(&[("cargo", receipt("rust", Some("tool"), "1.0.0", None))]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]);
    let report = run(&state, &cmd, &clock, &reg);

    let v = serde_json::to_value(&report).unwrap();
    assert_eq!(v["run_id"], "RUN01");
    assert_eq!(v["run_status"], "completed");
    assert_eq!(
        v["journal_seq"], 7,
        "the report pins the snapshot's log prefix"
    );
    assert_eq!(v["targets"][0]["outcome"], "matches");
    // `detail` is omitted for a clean match (skip_serializing_if).
    assert!(v["targets"][0].get("detail").is_none());
    assert_eq!(v["summary"]["matches"], 1);
}
