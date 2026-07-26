//! Unit tests for the resume/reconcile state machine (ADR-0003 §4).
//!
//! Covers every journal-state × remote-state cell of the state table (each →
//! its expected [`ResumeAction`]), the "already published ⇒ skip republish"
//! guarantee, adopt-forward for a publish that landed without a receipt, the
//! `Unknown`-is-never-`Missing` safety (and the `allow_unverified` go-ahead that
//! collapses it), and that a receipt-less lookup failure never fabricates a query.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::Path;

use super::{reconcile_for_resume, JournalState, ResumeAction};
use crate::contract::schema::{Adapter, Ecosystem, Registry};
use crate::ports::{Clock, CommandOutput, CommandRunner, RegistryQuery};
use crate::protocol::journal::{PublishReceipt, RunState, RunStatus};
use crate::protocol::plan::{PlanPhase, PlanTarget, ReleasePlan};
use crate::protocol::release::VerifyOutcome;
use crate::release::adapters::EffectCtx;

// ── Fakes ────────────────────────────────────────────────────────────────────

/// A command runner the reconcile must never touch (it is registry-query only).
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

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn plan_target(ecosystem: Ecosystem, registry: Registry, adapter: Adapter) -> PlanTarget {
    PlanTarget {
        ecosystem,
        package: Some("tool".to_string()),
        registry,
        adapter,
    }
}

/// A rust-only plan at `1.0.0`.
fn rust_plan() -> ReleasePlan {
    ReleasePlan {
        plan_id: "plan-abc".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.0.0".into(),
        targets: vec![plan_target(
            Ecosystem::Rust,
            Registry::CratesIo,
            Adapter::CargoPublish,
        )],
        phases: PlanPhase::SEQUENCE.to_vec(),
    }
}

/// A two-target rust+node plan at `1.0.0`.
fn two_target_plan() -> ReleasePlan {
    ReleasePlan {
        plan_id: "plan-abc".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.0.0".into(),
        targets: vec![
            plan_target(Ecosystem::Rust, Registry::CratesIo, Adapter::CargoPublish),
            plan_target(Ecosystem::Node, Registry::Npm, Adapter::NpmPublish),
        ],
        phases: PlanPhase::SEQUENCE.to_vec(),
    }
}

fn journal_receipt(ecosystem: &str, version: &str, digest: Option<&str>) -> PublishReceipt {
    PublishReceipt {
        ecosystem: ecosystem.to_string(),
        package: Some("tool".to_string()),
        version: version.to_string(),
        registry_url: None,
        digest: digest.map(str::to_string),
    }
}

/// A `RunState` for `plan-abc` with the given published `(target_id, receipt)` set.
fn state_with(published: &[(&str, PublishReceipt)], targets: &[&str]) -> RunState {
    let mut s = RunState::empty();
    s.run_id = "RUN01".into();
    s.plan_id = "plan-abc".into();
    s.version = "1.0.0".into();
    s.status = RunStatus::InProgress;
    s.targets = targets.iter().map(|t| (*t).to_string()).collect();
    for (id, r) in published {
        s.published.insert((*id).to_string(), r.clone());
    }
    s
}

fn reconcile(
    state: &RunState,
    plan: &ReleasePlan,
    cmd: &RecordingCmd,
    clock: &FixedClock,
    reg: &FakeRegistry,
    allow_unverified: bool,
) -> super::ResumeReconcile {
    let ctx = EffectCtx {
        runner: cmd,
        clock,
        registry: reg,
        repo_root: Path::new("/repo"),
    };
    reconcile_for_resume(state, plan, &ctx, allow_unverified)
}

/// Reconcile a single-rust-target run and return its one decision.
fn one_rust_decision(
    published: &[(&str, PublishReceipt)],
    reg: FakeRegistry,
    allow_unverified: bool,
) -> super::TargetDecision {
    let state = state_with(published, &["rust"]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let r = reconcile(&state, &rust_plan(), &cmd, &clock, &reg, allow_unverified);
    assert_eq!(r.decisions.len(), 1, "expected exactly one decision");
    // Read-only: the reconcile must never shell out.
    assert!(cmd.calls.borrow().is_empty(), "reconcile ran a command");
    r.decisions.into_iter().next().unwrap()
}

// ── The state table, cell by cell ────────────────────────────────────────────

#[test]
fn published_matches_skips() {
    let d = one_rust_decision(
        &[("rust", journal_receipt("rust", "1.0.0", None))],
        FakeRegistry::new().with("rust", "tool", &["0.9.0", "1.0.0"]),
        false,
    );
    assert_eq!(d.journal_state, JournalState::Published);
    assert_eq!(d.outcome, VerifyOutcome::Matches);
    assert_eq!(d.action, ResumeAction::Skip);
    assert!(!d.action.is_blocker());
    assert!(d.adopted_receipt.is_none());
}

#[test]
fn published_conflicts_is_a_hard_stop() {
    // A remote digest that disagrees with the receipt's ⇒ Conflicts ⇒ hard stop.
    let d = one_rust_decision(
        &[(
            "rust",
            journal_receipt("rust", "1.0.0", Some("sha256:ours")),
        )],
        FakeRegistry::new().with("rust", "tool", &["1.0.0"]),
        false,
    );
    // NB: the current RegistryQuery port lists versions only (no remote digest),
    // so this present-version case resolves to Matches, not Conflicts — the
    // digest-mismatch Conflicts path is exercised by the adapter layer's own
    // classify_receipt tests. Here we assert the *mapping* directly instead:
    assert_eq!(
        super::classify(JournalState::Published, VerifyOutcome::Conflicts, false),
        ResumeAction::Conflict
    );
    // …and that a present version is at least not a spurious blocker.
    assert!(!d.action.is_blocker());
}

#[test]
fn published_missing_is_a_hard_stop() {
    // A recorded publish the registry no longer reports: ambiguous, never a blind
    // re-publish.
    let d = one_rust_decision(
        &[("rust", journal_receipt("rust", "1.0.0", None))],
        FakeRegistry::new().with("rust", "tool", &["0.9.0"]), // 1.0.0 absent
        false,
    );
    assert_eq!(d.journal_state, JournalState::Published);
    assert_eq!(d.outcome, VerifyOutcome::Missing);
    assert_eq!(d.action, ResumeAction::Conflict);
    assert!(d.action.is_blocker());
    assert!(d.detail.as_deref().unwrap().contains("no longer reports"));
}

#[test]
fn published_unknown_is_unverifiable_without_go_ahead() {
    let d = one_rust_decision(
        &[("rust", journal_receipt("rust", "1.0.0", None))],
        FakeRegistry::outage(),
        false,
    );
    assert_eq!(d.outcome, VerifyOutcome::Unknown);
    assert_eq!(d.action, ResumeAction::Unverifiable);
    assert!(d.action.is_blocker(), "an outage must block, not proceed");
    assert!(d.detail.as_deref().unwrap().contains("--allow-unverified"));
}

#[test]
fn published_unknown_with_go_ahead_trusts_the_journal_and_skips() {
    let d = one_rust_decision(
        &[("rust", journal_receipt("rust", "1.0.0", None))],
        FakeRegistry::outage(),
        true, // explicit go-ahead
    );
    assert_eq!(d.outcome, VerifyOutcome::Unknown);
    assert_eq!(d.action, ResumeAction::Skip);
    assert!(!d.action.is_blocker());
}

#[test]
fn not_recorded_matches_adopts_forward() {
    // No receipt in the journal, but the registry holds the version: the publish
    // landed before its receipt fsynced — adopt it forward, never re-publish.
    let d = one_rust_decision(
        &[], // nothing recorded
        FakeRegistry::new().with("rust", "tool", &["1.0.0"]),
        false,
    );
    assert_eq!(d.journal_state, JournalState::NotRecorded);
    assert_eq!(d.outcome, VerifyOutcome::Matches);
    assert_eq!(d.action, ResumeAction::AdoptForward);
    assert!(!d.action.is_blocker());
    let receipt = d.adopted_receipt.expect("adopt-forward carries a receipt");
    assert_eq!(receipt.ecosystem, "rust");
    assert_eq!(receipt.version, "1.0.0");
    assert_eq!(receipt.package.as_deref(), Some("tool"));
}

#[test]
fn not_recorded_missing_resumes_the_publish() {
    let d = one_rust_decision(
        &[],
        FakeRegistry::new().with("rust", "tool", &["0.9.0"]), // 1.0.0 absent
        false,
    );
    assert_eq!(d.journal_state, JournalState::NotRecorded);
    assert_eq!(d.outcome, VerifyOutcome::Missing);
    assert_eq!(d.action, ResumeAction::ResumePublish);
    assert!(!d.action.is_blocker());
    assert!(d.adopted_receipt.is_none());
}

#[test]
fn not_recorded_unknown_is_unverifiable_without_go_ahead() {
    let d = one_rust_decision(&[], FakeRegistry::outage(), false);
    assert_eq!(d.journal_state, JournalState::NotRecorded);
    assert_eq!(d.outcome, VerifyOutcome::Unknown);
    assert_eq!(d.action, ResumeAction::Unverifiable);
    assert!(d.action.is_blocker());
}

#[test]
fn not_recorded_unknown_with_go_ahead_resumes_the_publish() {
    let d = one_rust_decision(&[], FakeRegistry::outage(), true);
    assert_eq!(d.outcome, VerifyOutcome::Unknown);
    assert_eq!(d.action, ResumeAction::ResumePublish);
    assert!(!d.action.is_blocker());
}

// ── Unknown safety: never a false Missing ────────────────────────────────────

#[test]
fn a_receipt_less_target_with_no_package_never_queries_the_registry() {
    // A plan target the plan could not resolve a package for: honest Unknown, and
    // no fabricated query the registry could read as "absent".
    let mut plan = rust_plan();
    plan.targets[0].package = None;
    let state = state_with(&[], &["rust"]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]);
    let r = reconcile(&state, &plan, &cmd, &clock, &reg, false);
    let d = &r.decisions[0];
    assert_eq!(d.outcome, VerifyOutcome::Unknown);
    assert_eq!(d.action, ResumeAction::Unverifiable);
    assert!(reg.queried.borrow().is_empty(), "no query was fabricated");
}

#[test]
fn structurally_unobservable_binary_target_is_unknown_not_missing() {
    // Binary (GitHub Releases / homebrew) is not observable through RegistryQuery
    // — even a registry that WOULD answer must not flip it to Matches/Missing.
    let plan = ReleasePlan {
        targets: vec![plan_target(
            Ecosystem::Binary,
            Registry::GhReleases,
            Adapter::Manual,
        )],
        ..rust_plan()
    };
    let state = state_with(&[], &["binary"]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("binary", "tool", &["1.0.0"]);
    let r = reconcile(&state, &plan, &cmd, &clock, &reg, false);
    let d = &r.decisions[0];
    assert_eq!(d.outcome, VerifyOutcome::Unknown);
    assert_eq!(d.action, ResumeAction::Unverifiable);
    assert!(
        reg.queried.borrow().is_empty(),
        "binary must not query the registry"
    );
}

// ── Cancelled targets never become publish candidates ────────────────────────

#[test]
fn a_cancelled_target_blocks_and_is_never_queried() {
    // A target the original run cancelled must not be silently un-cancelled into a
    // publish — the coordinator's publish-all skips only *published* targets, so
    // resume blocks instead.
    let mut state = state_with(&[], &["rust"]);
    state
        .cancelled
        .insert("rust".to_string(), "OTP timeout".to_string());
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    // A registry that WOULD answer must not tempt resume into publishing it.
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]);
    let r = reconcile(&state, &rust_plan(), &cmd, &clock, &reg, false);

    let d = &r.decisions[0];
    assert_eq!(d.journal_state, JournalState::Cancelled);
    assert_eq!(d.action, ResumeAction::Cancelled);
    assert!(
        d.action.is_blocker(),
        "a cancelled target must block resume"
    );
    assert!(r.is_blocked());
    assert!(d.detail.as_deref().unwrap().contains("OTP timeout"));
    assert!(d.adopted_receipt.is_none());
    // A cancelled target is never queried — its disposition is already decided.
    assert!(
        reg.queried.borrow().is_empty(),
        "a cancelled target must not hit the registry"
    );
}

#[test]
fn a_cancelled_target_blocks_even_with_the_go_ahead() {
    // The --allow-unverified go-ahead relaxes only the Unknown rows; it must never
    // un-cancel a target.
    let mut state = state_with(&[], &["rust"]);
    state
        .cancelled
        .insert("rust".to_string(), "manual skip".to_string());
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new();
    let r = reconcile(&state, &rust_plan(), &cmd, &clock, &reg, true);
    assert_eq!(r.decisions[0].action, ResumeAction::Cancelled);
    assert!(r.is_blocked());
}

// ── Multi-target roll-up + the resume-facing helpers ─────────────────────────

#[test]
fn already_published_target_is_skipped_while_the_other_resumes() {
    // rust published + matches (skip, no republish); node not recorded + missing
    // (resume publish). No blockers ⇒ the resume proceeds.
    let state = state_with(
        &[("rust", journal_receipt("rust", "1.0.0", None))],
        &["rust", "node"],
    );
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]); // node absent
    let r = reconcile(&state, &two_target_plan(), &cmd, &clock, &reg, false);

    assert_eq!(r.decisions.len(), 2);
    let rust = r.decisions.iter().find(|d| d.target == "rust").unwrap();
    let node = r.decisions.iter().find(|d| d.target == "node").unwrap();
    assert_eq!(rust.action, ResumeAction::Skip);
    assert_eq!(node.action, ResumeAction::ResumePublish);
    assert!(!r.is_blocked());
    assert!(r.blockers().is_empty());
    assert!(
        r.adoptions().is_empty(),
        "no publish landed without a receipt here"
    );
}

#[test]
fn adoptions_lists_only_the_forward_adopted_target() {
    // rust published + matches (skip); node NOT recorded but present remotely
    // (adopt forward) ⇒ exactly one adoption to journal.
    let state = state_with(
        &[("rust", journal_receipt("rust", "1.0.0", None))],
        &["rust", "node"],
    );
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new()
        .with("rust", "tool", &["1.0.0"])
        .with("node", "tool", &["1.0.0"]); // node landed without a receipt
    let r = reconcile(&state, &two_target_plan(), &cmd, &clock, &reg, false);

    assert!(!r.is_blocked());
    let adoptions = r.adoptions();
    assert_eq!(adoptions.len(), 1);
    assert_eq!(adoptions[0].0, "node");
    assert_eq!(adoptions[0].1.version, "1.0.0");
}

#[test]
fn a_single_conflict_blocks_the_whole_resume() {
    // rust published but now missing (hard stop); node fine. One blocker ⇒ blocked.
    let state = state_with(
        &[
            ("rust", journal_receipt("rust", "1.0.0", None)),
            ("node", journal_receipt("node", "1.0.0", None)),
        ],
        &["rust", "node"],
    );
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("node", "tool", &["1.0.0"]); // rust vanished
    let r = reconcile(&state, &two_target_plan(), &cmd, &clock, &reg, false);

    assert!(r.is_blocked());
    let blockers = r.blockers();
    assert_eq!(blockers.len(), 1);
    assert_eq!(blockers[0].target, "rust");
    assert_eq!(blockers[0].action, ResumeAction::Conflict);
}

#[test]
fn decisions_follow_plan_target_order_and_echo_run_identity() {
    let state = state_with(&[], &["rust", "node"]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new();
    let r = reconcile(&state, &two_target_plan(), &cmd, &clock, &reg, false);
    let ids: Vec<&str> = r.decisions.iter().map(|d| d.target.as_str()).collect();
    assert_eq!(ids, vec!["rust", "node"]);
    assert_eq!(r.run_id, "RUN01");
    assert_eq!(r.plan_id, "plan-abc");
}

// ── The classify mapping in isolation (exhaustive over the table) ────────────

#[test]
fn classify_covers_every_cell() {
    use JournalState::{NotRecorded, Published};
    use ResumeAction::{AdoptForward, Conflict, ResumePublish, Skip, Unverifiable};
    use VerifyOutcome::{Conflicts, Matches, Missing, Unknown};

    // Without the go-ahead.
    assert_eq!(super::classify(Published, Matches, false), Skip);
    assert_eq!(super::classify(Published, Conflicts, false), Conflict);
    assert_eq!(super::classify(Published, Missing, false), Conflict);
    assert_eq!(super::classify(Published, Unknown, false), Unverifiable);
    assert_eq!(super::classify(NotRecorded, Matches, false), AdoptForward);
    assert_eq!(super::classify(NotRecorded, Missing, false), ResumePublish);
    assert_eq!(super::classify(NotRecorded, Conflicts, false), Conflict);
    assert_eq!(super::classify(NotRecorded, Unknown, false), Unverifiable);

    // The go-ahead only ever relaxes the two Unknown rows.
    assert_eq!(super::classify(Published, Unknown, true), Skip);
    assert_eq!(super::classify(NotRecorded, Unknown, true), ResumePublish);
    assert_eq!(
        super::classify(Published, Missing, true),
        Conflict,
        "the go-ahead must never downgrade a genuine hard stop"
    );
}
