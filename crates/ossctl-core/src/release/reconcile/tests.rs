//! Unit tests for the read-only reconcile engine: every [`VerifyOutcome`] class
//! (including `Unknown`-on-lookup-failure and the structurally-unobservable
//! distribution targets), the roll-up summary, stable ordering, and a proof that
//! reconciling performs **no** command execution (read-only, registry query only).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::Path;

use super::reconcile;
use crate::ports::{Clock, CommandOutput, CommandRunner, RegistryQuery};
use crate::protocol::journal::{PublishReceipt, RunState, RunStatus};
use crate::protocol::release::VerifyOutcome;
use crate::release::adapters::EffectCtx;

// ── Fakes ────────────────────────────────────────────────────────────────────

/// Records every command it is asked to run — the reconcile must ask for none.
#[derive(Default)]
struct RecordingCmd {
    calls: RefCell<Vec<String>>,
}
impl CommandRunner for RecordingCmd {
    fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
        self.calls
            .borrow_mut()
            .push(format!("{program} {}", args.join(" ")));
        Ok(CommandOutput {
            status: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
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
        artifacts: &crate::release::adapters::ReleaseArtifacts::EMPTY,
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
fn unknown_for_unobservable_distribution_targets() {
    // Homebrew and GitHub Releases both journal under the `binary` ecosystem and
    // are not observable through RegistryQuery — even a registry that WOULD answer
    // must not flip them to Matches/Missing.
    let state = state_with(&[("gh", receipt("binary", Some("tool"), "1.0.0", None))]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("binary", "tool", &["1.0.0"]);
    let report = run(&state, &cmd, &clock, &reg);

    let t = only(&report);
    assert_eq!(t.outcome, VerifyOutcome::Unknown);
    assert!(t.detail.as_deref().unwrap().contains("not observable"));
    // The binary adapter reports Unknown without consulting the registry at all.
    assert!(
        reg.queried.borrow().is_empty(),
        "binary must not query the registry"
    );
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
        ("gh", receipt("binary", Some("tool"), "1.0.0", None)), // Unknown (unobservable)
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
    assert_eq!(report.summary.matches, 1);
    assert_eq!(report.summary.missing, 1);
    assert_eq!(report.summary.unknown, 1);
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
fn reconcile_runs_no_commands() {
    // Read-only: the reconcile consults the registry port only — it must never
    // shell out through the CommandRunner.
    let state = state_with(&[
        ("cargo", receipt("rust", Some("tool"), "1.0.0", None)),
        ("gh", receipt("binary", Some("tool"), "1.0.0", None)),
    ]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]);
    let _ = run(&state, &cmd, &clock, &reg);

    assert!(
        cmd.calls.borrow().is_empty(),
        "reconcile must not execute any command"
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
