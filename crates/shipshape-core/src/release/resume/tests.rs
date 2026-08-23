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
use crate::protocol::journal::{
    Phase, PhaseOutcome, PhaseRecord, PublishReceipt, RunState, RunStatus,
};
use crate::protocol::plan::{PlanPhase, PlanTarget, ReleasePlan};
use crate::protocol::release::VerifyOutcome;
use crate::release::adapters::EffectCtx;

// ── Fakes ────────────────────────────────────────────────────────────────────

/// Records destination-observation commands used by distribution targets.
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec![
            "aarch64-apple-darwin".into(),
            "aarch64-unknown-linux-musl".into(),
            "x86_64-unknown-linux-musl".into(),
        ],
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
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec![
            "aarch64-apple-darwin".into(),
            "aarch64-unknown-linux-musl".into(),
            "x86_64-unknown-linux-musl".into(),
        ],
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
///
/// The run is placed **in the publish phase** (`current_phase = Publish`) — the
/// realistic resume scenario (a crash during/after `publish-all`, where a publish
/// could have landed before its receipt fsynced). This keeps the not-recorded
/// `Unknown` row `Unverifiable` without an explicit go-ahead. For a run that failed
/// *before* publish was ever reached, use [`state_before_publish`].
fn state_with(published: &[(&str, PublishReceipt)], targets: &[&str]) -> RunState {
    let mut s = RunState::empty();
    s.run_id = "RUN01".into();
    s.plan_id = "plan-abc".into();
    s.version = "1.0.0".into();
    s.status = RunStatus::InProgress;
    s.targets = targets.iter().map(|t| (*t).to_string()).collect();
    s.current_phase = Some(Phase::Publish);
    s.phases = vec![
        PhaseRecord {
            phase: Phase::DryRun,
            outcome: PhaseOutcome::Ok,
        },
        PhaseRecord {
            phase: Phase::Build,
            outcome: PhaseOutcome::Ok,
        },
    ];
    for (id, r) in published {
        s.published.insert((*id).to_string(), r.clone());
    }
    s
}

/// Like [`state_with`], but for a run that failed in the **build** phase — the
/// publish phase was never reached, so nothing could have published without a
/// receipt. (`current_phase = Build`, and no phase record is `Publish` or later.)
///
/// Deliberately takes **no** published receipts: a run that never reached publish
/// cannot hold one, and a receipt is itself irrefutable proof publish ran
/// (`publish_phase_reached`), so an internally-contradictory fixture must not be
/// constructible through this helper.
fn state_before_publish(targets: &[&str]) -> RunState {
    let mut s = state_with(&[], targets);
    s.current_phase = Some(Phase::Build);
    s.phases = vec![PhaseRecord {
        phase: Phase::DryRun,
        outcome: PhaseOutcome::Ok,
    }];
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
        artifacts: &crate::release::adapters::EMPTY_ARTIFACTS,
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

/// Like [`one_rust_decision`], but for a run that failed **before** the publish
/// phase was reached (a build-phase failure).
fn one_rust_decision_before_publish(
    reg: FakeRegistry,
    allow_unverified: bool,
) -> super::TargetDecision {
    let state = state_before_publish(&["rust"]);
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let r = reconcile(&state, &rust_plan(), &cmd, &clock, &reg, allow_unverified);
    assert_eq!(r.decisions.len(), 1, "expected exactly one decision");
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
        super::classify(
            JournalState::Published,
            VerifyOutcome::Conflicts,
            false,
            true
        ),
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

// ── Publish-phase-reached refinement of (not recorded, Unknown) ───────────────

#[test]
fn not_recorded_unknown_resumes_without_go_ahead_when_publish_never_reached() {
    // The run failed in the BUILD phase (publish-all never started), and the target
    // is a rust/cargo target with no wired registry query (Unknown). Nothing could
    // have published, so resume must proceed WITHOUT --allow-unverified. This is the
    // exact scenario the resume-publish-phase-never-reached bug reported.
    let d = one_rust_decision_before_publish(FakeRegistry::outage(), false);
    assert_eq!(d.journal_state, JournalState::NotRecorded);
    assert_eq!(d.outcome, VerifyOutcome::Unknown);
    assert_eq!(d.action, ResumeAction::ResumePublish);
    assert!(
        !d.action.is_blocker(),
        "a build-phase failure must not demand --allow-unverified"
    );
    assert!(d.adopted_receipt.is_none());
    assert!(d
        .detail
        .as_deref()
        .unwrap()
        .contains("publish phase was never reached"));
}

#[test]
fn not_recorded_unknown_stays_unverifiable_mid_publish_crash() {
    // The mid-publish crash: publish-all WAS entered (a publish could have landed
    // before its receipt fsynced), the target verifies Unknown, and there is no
    // go-ahead. This must STILL block — the publish-never-reached relaxation must
    // not leak into this genuinely-unsafe row.
    let d = one_rust_decision(&[], FakeRegistry::outage(), false);
    assert_eq!(d.journal_state, JournalState::NotRecorded);
    assert_eq!(d.outcome, VerifyOutcome::Unknown);
    assert_eq!(d.action, ResumeAction::Unverifiable);
    assert!(
        d.action.is_blocker(),
        "a mid-publish crash must still demand --allow-unverified"
    );
}

#[test]
fn partial_publish_all_keeps_the_unpublished_unknown_target_unverifiable() {
    // The ADR-0003 §4 driver scenario: publish-all published rust (a receipt lands),
    // then crashed before node. node is (NotRecorded, Unknown). Because the run
    // provably reached publish (rust's receipt), node must STILL block without a
    // go-ahead — the publish-never-reached relaxation must not fire for a run that
    // clearly reached publish.
    let state = state_with(
        &[("rust", journal_receipt("rust", "1.0.0", None))],
        &["rust", "node"],
    );
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::outage(); // node verifies Unknown
    let r = reconcile(&state, &two_target_plan(), &cmd, &clock, &reg, false);

    let node = r.decisions.iter().find(|d| d.target == "node").unwrap();
    assert_eq!(node.journal_state, JournalState::NotRecorded);
    assert_eq!(node.outcome, VerifyOutcome::Unknown);
    assert_eq!(node.action, ResumeAction::Unverifiable);
    assert!(
        r.is_blocked(),
        "a reached-publish run must not silently resume"
    );
}

#[test]
fn a_landed_receipt_proves_publish_reached_even_if_phase_records_regressed() {
    // The defensive core of the fix: even if the durable phase bookkeeping is lost
    // or rewound (a crash between a registry side-effect and its phase fsync, a v1
    // journal, a journal partially reconstructed under remote-is-ground-truth
    // resume), a landed receipt is irrefutable proof publish ran. A co-target that
    // is (NotRecorded, Unknown) must therefore STILL block — the publish-never-
    // reached relaxation must not leak in through lost phase records.
    let mut state = state_with(
        &[("rust", journal_receipt("rust", "1.0.0", None))],
        &["rust", "node"],
    );
    // Simulate lost/rewound phase bookkeeping: no publish-or-later phase signal.
    state.current_phase = Some(Phase::Build);
    state.phases = vec![PhaseRecord {
        phase: Phase::DryRun,
        outcome: PhaseOutcome::Ok,
    }];

    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::outage(); // node verifies Unknown
    let r = reconcile(&state, &two_target_plan(), &cmd, &clock, &reg, false);

    let node = r.decisions.iter().find(|d| d.target == "node").unwrap();
    assert_eq!(node.action, ResumeAction::Unverifiable);
    assert!(
        r.is_blocked(),
        "a receipt proves publish ran, so an Unknown co-target must not resume blind"
    );
}

#[test]
fn publish_phase_reached_recognizes_every_signal_and_the_pristine_pre_publish_state() {
    // Pristine pre-publish run: only DryRun/Build signals ⇒ not reached.
    let mut s = state_before_publish(&["rust"]);
    assert!(!super::publish_phase_reached(&s));

    // Each publish-or-later phase signal, on its own, is enough.
    s.current_phase = Some(Phase::Publish);
    assert!(super::publish_phase_reached(&s));
    s.current_phase = Some(Phase::Build);
    s.phases.push(PhaseRecord {
        phase: Phase::Publish,
        outcome: PhaseOutcome::Failed,
    });
    assert!(super::publish_phase_reached(&s));

    // Each durable *effect* is irrefutable proof, even with no phase signal at all.
    let effect_only = || {
        let mut s = state_before_publish(&["rust"]);
        s.current_phase = None;
        s.phases.clear();
        s
    };
    let mut with_receipt = effect_only();
    with_receipt
        .published
        .insert("rust".into(), journal_receipt("rust", "1.0.0", None));
    assert!(super::publish_phase_reached(&with_receipt));

    let mut with_delegation = effect_only();
    with_delegation.delegated.insert("rust".into());
    assert!(super::publish_phase_reached(&with_delegation));

    let mut with_tag = effect_only();
    with_tag.tags.insert(
        "v1.0.0".into(),
        crate::protocol::journal::TagState::default(),
    );
    assert!(super::publish_phase_reached(&with_tag));
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
fn allow_unverified_skips_only_unknown_never_missing_or_conflicts() {
    assert_eq!(
        super::classify(JournalState::Published, VerifyOutcome::Unknown, true, true,),
        ResumeAction::Skip,
        "the explicit go-ahead may trust only an unobservable receipt"
    );
    for outcome in [VerifyOutcome::Missing, VerifyOutcome::Conflicts] {
        assert_eq!(
            super::classify(JournalState::Published, outcome, true, true),
            ResumeAction::Conflict,
            "--allow-unverified must not bypass a concrete {outcome:?} result"
        );
    }
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

// ── CI-delegated targets are non-blocking skips, never publish candidates ─────

/// A rust plan whose single target is the CI-delegated `cargo-dist` adapter.
fn cargo_dist_plan() -> ReleasePlan {
    ReleasePlan {
        plan_id: "plan-abc".into(),
        contract_schema_version: 1,
        head_sha: "deadbeef".into(),
        version: "1.0.0".into(),
        targets: vec![plan_target(
            Ecosystem::Rust,
            Registry::GhReleases,
            Adapter::CargoDist,
        )],
        phases: PlanPhase::SEQUENCE.to_vec(),
        bump: None,
        homebrew_tap: None,
        license: None,
        description: Some("Test release tool".into()),
        homebrew_platforms: vec![
            "aarch64-apple-darwin".into(),
            "aarch64-unknown-linux-musl".into(),
            "x86_64-unknown-linux-musl".into(),
        ],
    }
}

#[test]
fn a_journalled_delegated_target_is_a_non_blocking_skip() {
    // The original run recorded `target_delegated`; resume classifies it Delegated
    // (non-blocking) and never queries a registry that cannot observe it.
    let mut state = state_with(&[], &["rust"]);
    state.delegated.insert("rust".to_string());
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new().with("rust", "tool", &["1.0.0"]);
    let r = reconcile(&state, &cargo_dist_plan(), &cmd, &clock, &reg, false);

    let d = &r.decisions[0];
    assert_eq!(d.journal_state, JournalState::Delegated);
    assert_eq!(d.action, ResumeAction::Delegated);
    assert!(
        !d.action.is_blocker(),
        "a delegated target must not block resume"
    );
    assert!(!r.is_blocked());
    assert!(d.adopted_receipt.is_none());
    assert!(
        reg.queried.borrow().is_empty(),
        "a delegated target must not hit the registry"
    );
}

#[test]
fn a_delegated_target_is_recognized_by_adapter_capability_without_a_journal_event() {
    // The load-bearing case: a run that failed on cargo-dist's `Unsupported` BEFORE
    // `target_delegated` existed (a v1 journal, or a crash before the append). The
    // journal has no delegation fact, but the resolved adapter still declares itself
    // CI-delegated — so resume must NOT treat it as a not-recorded publish candidate
    // (which would drive a spurious re-publish attempt), it stays a Delegated skip.
    let state = state_with(&[], &["rust"]); // note: no `delegated` entry
    let (cmd, clock) = (RecordingCmd::default(), FixedClock);
    let reg = FakeRegistry::new();
    let r = reconcile(&state, &cargo_dist_plan(), &cmd, &clock, &reg, false);

    let d = &r.decisions[0];
    assert_eq!(d.journal_state, JournalState::Delegated);
    assert_eq!(d.action, ResumeAction::Delegated);
    assert!(!r.is_blocked());
    assert!(
        reg.queried.borrow().is_empty(),
        "a capability-delegated target must not hit the registry"
    );
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

    // The state table with publish reached (the resume-time default) and no
    // go-ahead. `reached` is the publish_phase_reached signal.
    let reached = true;
    assert_eq!(super::classify(Published, Matches, false, reached), Skip);
    assert_eq!(
        super::classify(Published, Conflicts, false, reached),
        Conflict
    );
    assert_eq!(
        super::classify(Published, Missing, false, reached),
        Conflict
    );
    assert_eq!(
        super::classify(Published, Unknown, false, reached),
        Unverifiable
    );
    assert_eq!(
        super::classify(NotRecorded, Matches, false, reached),
        AdoptForward
    );
    assert_eq!(
        super::classify(NotRecorded, Missing, false, reached),
        ResumePublish
    );
    assert_eq!(
        super::classify(NotRecorded, Conflicts, false, reached),
        Conflict
    );
    assert_eq!(
        super::classify(NotRecorded, Unknown, false, reached),
        Unverifiable,
        "mid-publish crash (publish reached, no receipt) stays unverifiable"
    );

    // The go-ahead relaxes the two Unknown rows.
    assert_eq!(super::classify(Published, Unknown, true, reached), Skip);
    assert_eq!(
        super::classify(NotRecorded, Unknown, true, reached),
        ResumePublish
    );
    assert_eq!(
        super::classify(Published, Missing, true, reached),
        Conflict,
        "the go-ahead must never downgrade a genuine hard stop"
    );

    // Publish phase NEVER reached: only the (not recorded, Unknown) cell changes —
    // it resolves to ResumePublish even without the go-ahead (nothing could have
    // published). Every other cell is invariant under this signal.
    let never = false;
    assert_eq!(
        super::classify(NotRecorded, Unknown, false, never),
        ResumePublish,
        "publish never reached ⇒ nothing could have published ⇒ resume, no go-ahead"
    );
    assert_eq!(
        super::classify(NotRecorded, Unknown, true, never),
        ResumePublish
    );
    // The Published × Unknown row must NOT be relaxed by the never-reached signal
    // (a receipt implies publish ran; the signal never legitimately co-occurs, but
    // the mapping must stay safe even if it did).
    assert_eq!(
        super::classify(Published, Unknown, false, never),
        Unverifiable,
        "published × Unknown is never relaxed by the publish-phase signal"
    );
    assert_eq!(super::classify(Published, Missing, false, never), Conflict);
    assert_eq!(
        super::classify(NotRecorded, Matches, false, never),
        AdoptForward
    );
    assert_eq!(
        super::classify(NotRecorded, Missing, false, never),
        ResumePublish
    );
}
