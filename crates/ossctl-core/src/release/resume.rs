//! Remote-is-ground-truth resume/reconcile (ADR-0003 §4).
//!
//! `release resume` continues an interrupted run — but it does **not** trust the
//! local journal as authoritative for what actually published. The journal is an
//! optimization; the **remote registry state is the ground truth** (a run whose
//! `.git`-local journal was lost can still be reconciled from what the registries
//! hold, via each adapter's [`verify`](ReleaseAdapter::verify)). This module is
//! the read-only *reconcile* half: it classifies every planned target against the
//! ADR-0003 §4 state table and returns the per-target action a resume must take.
//! Actually continuing the phase barrier is the [coordinator](super::coordinator)'s
//! job — this module never mutates the journal or the registry; it only decides.
//!
//! # The state table (ADR-0003 §4)
//!
//! For each target the run planned, its journal state (does a durable
//! [`PublishReceipt`](crate::protocol::journal::PublishReceipt) exist?) is crossed
//! with what [`verify`](ReleaseAdapter::verify) observes remotely:
//!
//! | Journal | `verify()` | Action ([`ResumeAction`]) |
//! |---|---|---|
//! | published | `Matches` | [`Skip`](ResumeAction::Skip) — done, idempotent success |
//! | published | `Conflicts` | [`Conflict`](ResumeAction::Conflict) — hard stop, never overwrite |
//! | published | `Missing` | [`Conflict`](ResumeAction::Conflict) — ambiguous, hard stop + surface |
//! | published | `Unknown` | [`Unverifiable`](ResumeAction::Unverifiable) — needs explicit go-ahead |
//! | not recorded | `Matches` | [`AdoptForward`](ResumeAction::AdoptForward) — publish landed pre-receipt; adopt it |
//! | not recorded | `Missing` | [`ResumePublish`](ResumeAction::ResumePublish) — resume the publish |
//! | not recorded | `Unknown`, publish phase reached | [`Unverifiable`](ResumeAction::Unverifiable) — a publish could have landed pre-receipt; needs explicit go-ahead |
//! | not recorded | `Unknown`, publish phase **never** reached | [`ResumePublish`](ResumeAction::ResumePublish) — nothing could have published; resume the publish |
//!
//! The `Unknown` rows are the tri-state discipline (also ADR-0002 §1): a lookup
//! that **could not be performed** — a registry outage, a package with no name, an
//! ecosystem this binary cannot query, or a structurally-unobservable distribution
//! target (homebrew taps / GitHub Releases) — is **never** read as `Missing` (which
//! would drive a dangerous blind re-publish of an already-published version). When a
//! receipt exists (`published × Unknown`) it is surfaced as unverifiable; a resume
//! proceeds past it only with an explicit human go-ahead (`allow_unverified`), which
//! collapses `Unknown` to trust-the-journal (`Skip`) rather than a hard stop.
//!
//! For a **not-recorded** target the `Unknown` disposition is refined by whether the
//! run ever entered the publish phase (`publish_phase_reached`, derived from
//! [`RunState`]): if publish was **never reached** (the run failed in dry-run/build),
//! nothing could have published without a receipt, so the cell resolves directly to
//! `ResumePublish` — no go-ahead needed. Only when publish *was* reached (a crash
//! mid-`publish-all`, where a publish could have landed before its receipt fsynced)
//! does it stay `Unverifiable` pending the `allow_unverified` go-ahead. This never
//! touches the `published × Unknown` row: a receipt implies publish ran.
//!
//! The **tag** rows of the ADR table (`created_local` only → retry push;
//! `pushed_remote`, no Release → create Release) are *not* reconciled here: the
//! coordinator's tag-once phase is already an idempotent, step-by-step re-entry
//! (each of `tag_created_local` / `tag_pushed_remote` / `github_release_created`
//! is skipped if journalled and the [`Tagger`](crate::ports::Tagger) treats
//! "already exists" as success), so continuing the barrier *is* the tag reconcile.
//! Forking a second copy of that logic here is exactly what ADR-0003 forbids.
//!
//! A target the original run **cancelled** (a `target_cancelled` fact) is off the
//! table entirely: it is a deliberate skip, and the coordinator's publish-all skips
//! only *published* targets, so continuing would re-publish it. Resume classifies it
//! as [`ResumeAction::Cancelled`] — a hard stop — rather than silently un-cancelling
//! it (there is no ADR-0003 cell for cancelled × remote).

use std::collections::HashMap;

use crate::contract::schema::Ecosystem;
use crate::protocol::journal::{Phase, PublishReceipt as JournalReceipt, RunState};
use crate::protocol::plan::{PlanTarget, ReleasePlan};
use crate::protocol::release::{PublishReceipt as AdapterReceipt, VerifyOutcome};

use super::adapters::{resolve, EffectCtx, ReleaseAdapter};

/// Whether a target carried a durable publish receipt in the journal at reconcile
/// time — the left axis of the ADR-0003 §4 state table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalState {
    /// The journal holds a `target_published` receipt for this target.
    Published,
    /// The journal holds no receipt for this target (never published, or the
    /// publish landed before its receipt was fsynced).
    NotRecorded,
    /// The journal recorded a `target_cancelled` for this target — a deliberate
    /// skip. Resume must never silently un-cancel it into a publish.
    Cancelled,
    /// The journal recorded a `target_delegated` for this target — its artifact is
    /// produced by the tag-triggered CI, not the engine (e.g. `cargo-dist`). Resume
    /// must not try to publish it; there is nothing for the engine to resume.
    Delegated,
}

/// The reconciled action for one target — the resolved cell of the ADR-0003 §4
/// state table (journal-state × remote-state), with the `Unknown` rows already
/// collapsed by the caller's `allow_unverified` go-ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeAction {
    /// published × `Matches` — already landed; the coordinator's publish-all skips
    /// it (it is in `state.published`). Nothing to do.
    Skip,
    /// not-recorded × `Matches` — a publish landed before its receipt fsynced;
    /// **adopt the receipt forward** (journal a `target_published`) so the
    /// coordinator skips it rather than re-publishing an already-published version.
    AdoptForward,
    /// not-recorded × `Missing` — the publish genuinely did not land; let the
    /// coordinator resume it in publish-all.
    ResumePublish,
    /// published × {`Conflicts`, `Missing`} — a **hard stop**: something other than
    /// this run's artifact is at that version, or a recorded publish has vanished.
    /// Never overwritten, never blind-re-published; surfaced for a human.
    Conflict,
    /// `Unknown` with no explicit go-ahead — the reconcile could not be performed,
    /// so the target is **unverifiable**. A hard stop until a human passes the
    /// go-ahead (`allow_unverified`), because an outage must never be assumed to
    /// mean "not published".
    Unverifiable,
    /// The target was cancelled in the original run — a **hard stop**. The
    /// coordinator's publish-all skips only *published* targets, so continuing
    /// would re-publish a target the operator deliberately cancelled; resume never
    /// silently un-cancels it (there is no ADR-0003 cell for cancelled × remote).
    Cancelled,
    /// The target is **CI-delegated** — its artifact is produced by the
    /// tag-triggered CI, not the engine, so there is nothing for a resume to
    /// publish. **Not** a blocker: the coordinator re-journals/skips it on re-entry,
    /// exactly as on a fresh cut.
    Delegated,
}

impl ResumeAction {
    /// Whether this action **blocks** a resume (a hard stop that must be surfaced,
    /// not continued past).
    #[must_use]
    pub fn is_blocker(self) -> bool {
        matches!(self, Self::Conflict | Self::Unverifiable | Self::Cancelled)
    }

    /// The stable wire/diagnostic string for this action.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::AdoptForward => "adopt_forward",
            Self::ResumePublish => "resume_publish",
            Self::Conflict => "conflict",
            Self::Unverifiable => "unverifiable",
            Self::Cancelled => "cancelled",
            Self::Delegated => "delegated",
        }
    }
}

/// One target's reconcile decision — the classified cell plus the material a
/// resume needs to act on it.
#[derive(Debug, Clone)]
pub struct TargetDecision {
    /// The journal/coordinator target id — the ecosystem wire string for a
    /// lone-in-its-ecosystem target, else a per-target key
    /// ([`journal_target_ids`](crate::release::journal_target_ids)).
    pub target: String,
    /// The ecosystem this target publishes to.
    pub ecosystem: Ecosystem,
    /// Whether the journal recorded a receipt for it.
    pub journal_state: JournalState,
    /// What the adapter's `verify` observed remotely.
    pub outcome: VerifyOutcome,
    /// The resolved action from the state table.
    pub action: ResumeAction,
    /// An operator-facing reason for a non-`Skip` decision (why it conflicts, is
    /// unverifiable, will be adopted, or will be resumed).
    pub detail: Option<String>,
    /// For [`ResumeAction::AdoptForward`], the synthetic receipt to journal so the
    /// coordinator treats the target as already published. `None` otherwise.
    pub adopted_receipt: Option<JournalReceipt>,
}

/// The full reconcile of a run against remote registry state — one
/// [`TargetDecision`] per planned target, in the plan's target order.
#[derive(Debug, Clone)]
pub struct ResumeReconcile {
    /// The run reconciled.
    pub run_id: String,
    /// The sealed plan id the run executes.
    pub plan_id: String,
    /// One decision per planned target.
    pub decisions: Vec<TargetDecision>,
}

impl ResumeReconcile {
    /// The decisions that **block** the resume (hard stops the caller must surface
    /// via the §10 envelope rather than continue past).
    #[must_use]
    pub fn blockers(&self) -> Vec<&TargetDecision> {
        self.decisions
            .iter()
            .filter(|d| d.action.is_blocker())
            .collect()
    }

    /// Whether any decision blocks the resume.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        self.decisions.iter().any(|d| d.action.is_blocker())
    }

    /// The `(target id, receipt)` pairs to journal as `target_published` **before**
    /// continuing the barrier, so an adopted-forward publish is never re-run. Empty
    /// unless a publish landed without a durable receipt (not-recorded × `Matches`).
    #[must_use]
    pub fn adoptions(&self) -> Vec<(&str, &JournalReceipt)> {
        self.decisions
            .iter()
            .filter_map(|d| d.adopted_receipt.as_ref().map(|r| (d.target.as_str(), r)))
            .collect()
    }
}

/// Reconcile a journaled run against current remote registry state, per the
/// ADR-0003 §4 state table.
///
/// Read-only with respect to the world **except** for the registry lookups it
/// performs through `ctx` (the same read-only `verify` path `release verify` uses)
/// — it writes nothing to the journal or the registry. Iterates `plan.targets` (the
/// authority for the run's target set; the caller has already confirmed the plan
/// still hashes to the run's `plan_id`), classifies each cell, and returns the
/// per-target [`TargetDecision`]s.
///
/// `allow_unverified` is the human's explicit go-ahead for the `Unknown` rows: with
/// it, an unverifiable target is trusted to the journal (`Skip` when a receipt
/// exists, `ResumePublish` when not) instead of blocking. It never downgrades a
/// genuine `Conflicts`/`Missing`-after-publish hard stop.
#[must_use]
pub fn reconcile_for_resume(
    state: &RunState,
    plan: &ReleasePlan,
    ctx: &EffectCtx<'_>,
    allow_unverified: bool,
) -> ResumeReconcile {
    // The remote outcome for *published* targets comes from the same read-only
    // reconcile engine `release verify` uses (remote is ground truth). Note this
    // supplies only the outcome — the journal-state axis is decided directly from
    // `state.published` below, never from report membership, so a receipt can never
    // be misclassified as not-recorded (which would risk a double publish).
    let published_report = super::reconcile::reconcile(state, ctx);
    let published: HashMap<&str, (VerifyOutcome, Option<String>)> = published_report
        .targets
        .iter()
        .map(|t| (t.target.as_str(), (t.outcome, t.detail.clone())))
        .collect();

    // The same per-target journal ids the coordinator keyed `state.published` by
    // (and the CLI journalled as `RunCreated.targets`) — derived from the same
    // plan, so a resume looks up the right receipt for every target even when an
    // ecosystem carries several (never the bare ecosystem, which would collide and
    // risk re-publishing an already-landed crate).
    let target_ids = super::journal_target_ids(&plan.targets);
    // A run-level fact: did this run ever enter the publish phase? Before that
    // point nothing could have landed on a registry, so a not-recorded target that
    // verifies `Unknown` (an unqueryable ecosystem, e.g. rust/cargo) is safe to
    // resume without the `--allow-unverified` go-ahead — the "publish never
    // reached" refinement of the ADR-0003 §4 `(not recorded, Unknown)` cell.
    let publish_reached = publish_phase_reached(state);
    let mut decisions = Vec::with_capacity(plan.targets.len());
    for (pt, target) in plan.targets.iter().zip(target_ids) {
        // A cancelled target is a deliberate skip, not a publish candidate. The
        // coordinator's publish-all skips only *published* targets, so continuing
        // would re-publish it — block instead of silently un-cancelling.
        if let Some(reason) = state.cancelled.get(&target) {
            decisions.push(TargetDecision {
                target,
                ecosystem: pt.ecosystem,
                journal_state: JournalState::Cancelled,
                outcome: VerifyOutcome::Unknown,
                action: ResumeAction::Cancelled,
                detail: Some(format!(
                    "this target was cancelled in the original run ({reason}); resuming would \
                     re-publish it. ossctl will not silently un-cancel a target — abandon and \
                     re-plan, or reconcile it by hand"
                )),
                adopted_receipt: None,
            });
            continue;
        }

        // A CI-delegated target is off the publish table: the tag-triggered CI owns
        // its artifact, so there is nothing for the engine to resume. Classify it as
        // a non-blocking Delegated skip rather than querying a registry that cannot
        // observe it (which would misread as `Missing` → a spurious re-publish).
        //
        // Delegation is decided by EITHER the journal (`target_delegated` was
        // recorded) OR the adapter's live capability. The latter is load-bearing for
        // two cases the journal alone misses: a **v1** run that failed on this
        // adapter's `Unsupported` before the event existed, and a crash after the
        // publish phase entered but before `target_delegated` was appended. In both,
        // the resolved adapter still declares itself delegated, so resume never tries
        // to publish it.
        if state.delegated.contains(&target) || resolve(pt.adapter).is_ci_delegated() {
            decisions.push(TargetDecision {
                target,
                ecosystem: pt.ecosystem,
                journal_state: JournalState::Delegated,
                outcome: VerifyOutcome::Unknown,
                action: ResumeAction::Delegated,
                detail: Some(
                    "this target is produced by the tag-triggered CI (delegated), not the \
                     engine; there is nothing to resume"
                        .to_string(),
                ),
                adopted_receipt: None,
            });
            continue;
        }

        // The journal-state axis: authoritative from `state.published`.
        let (journal_state, outcome, verify_detail) = if state.published.contains_key(&target) {
            // A receipt exists; take its remote outcome from the reconcile report
            // (defensively Unknown if — impossibly — the engine omitted the row).
            let (outcome, detail) = published.get(target.as_str()).cloned().unwrap_or((
                VerifyOutcome::Unknown,
                Some("the published receipt could not be reconciled against the registry".into()),
            ));
            (JournalState::Published, outcome, detail)
        } else {
            let (outcome, detail) = verify_not_recorded(ctx, pt, &plan.version);
            (JournalState::NotRecorded, outcome, detail)
        };

        let action = classify(journal_state, outcome, allow_unverified, publish_reached);
        let adopted_receipt = (action == ResumeAction::AdoptForward).then(|| JournalReceipt {
            ecosystem: pt.ecosystem.as_str().to_string(),
            package: pt.package.clone(),
            version: plan.version.clone(),
            // The current RegistryQuery port lists versions only (no remote digest
            // or URL to capture); an adopted receipt therefore records presence,
            // matching what a live publish receipt carries through this port. A
            // richer digest-observing port is a documented follow-up.
            registry_url: None,
            digest: None,
        });
        decisions.push(TargetDecision {
            detail: action_detail(
                action,
                outcome,
                journal_state,
                publish_reached,
                verify_detail,
            ),
            target,
            ecosystem: pt.ecosystem,
            journal_state,
            outcome,
            action,
            adopted_receipt,
        });
    }

    ResumeReconcile {
        run_id: state.run_id.clone(),
        plan_id: state.plan_id.clone(),
        decisions,
    }
}

/// Whether the run ever entered the publish phase — the point at or after which a
/// target could have landed on a registry without its receipt fsyncing (a crash
/// mid-`publish-all`). Derived from [`RunState`]: the phase currently in progress
/// is [`Phase::Publish`] or later, OR any recorded phase barrier is `Publish` or
/// later. `Phase`'s `Ord` follows barrier order (`DryRun < Build < Publish < Tag <
/// Dist`), so `>= Phase::Publish` is exactly "publish-or-beyond".
///
/// Before that point nothing could have published, so a not-recorded target that
/// verifies `Unknown` is safe to resume without the `--allow-unverified` go-ahead
/// (see `classify`). This never affects the `Published` rows: a receipt only
/// exists because publish ran, so `Published` already implies publish was reached.
fn publish_phase_reached(state: &RunState) -> bool {
    state.current_phase.is_some_and(|p| p >= Phase::Publish)
        || state.phases.iter().any(|r| r.phase >= Phase::Publish)
}

/// Map one (journal-state × remote-outcome) cell to its [`ResumeAction`], folding
/// the `allow_unverified` go-ahead and the `publish_phase_reached` signal into the
/// `(not recorded, Unknown)` row.
// Each arm is one cell of the ADR-0003 §4 state table, kept separate (even where
// two resolve to the same action) so the mapping reads as the documented table and
// a future divergence is a one-line edit, not a pattern split.
#[allow(clippy::match_same_arms)]
fn classify(
    journal_state: JournalState,
    outcome: VerifyOutcome,
    allow_unverified: bool,
    publish_phase_reached: bool,
) -> ResumeAction {
    use JournalState::{Cancelled, Delegated, NotRecorded, Published};
    use VerifyOutcome::{Conflicts, Matches, Missing, Unknown};
    match (journal_state, outcome) {
        // Cancelled / delegated targets are decided before classify (never queried);
        // these arms only satisfy exhaustiveness and mirror that disposition.
        (Cancelled, _) => ResumeAction::Cancelled,
        (Delegated, _) => ResumeAction::Delegated,
        (Published, Matches) => ResumeAction::Skip,
        // A recorded publish that now conflicts, or has vanished, is a hard stop:
        // never overwrite someone else's artifact, never blind-re-publish.
        (Published, Conflicts | Missing) => ResumeAction::Conflict,
        (Published, Unknown) => {
            // NB: a receipt only exists because publish ran, so `publish_phase_reached`
            // is necessarily true here — this row is never relaxed by that signal.
            if allow_unverified {
                // The go-ahead trusts the journal's own receipt.
                ResumeAction::Skip
            } else {
                ResumeAction::Unverifiable
            }
        }
        // A publish landed before its receipt fsynced — adopt it forward.
        (NotRecorded, Matches) => ResumeAction::AdoptForward,
        // Genuinely absent remotely — resume the publish.
        (NotRecorded, Missing) => ResumeAction::ResumePublish,
        // Cannot arise from a receipt-less query (no local digest to disagree), but
        // classify it as a hard stop rather than guess if a future port surfaces it.
        (NotRecorded, Conflicts) => ResumeAction::Conflict,
        (NotRecorded, Unknown) => {
            if !publish_phase_reached {
                // The publish phase was provably never entered (the run failed in
                // dry-run/build), so nothing could have published without a
                // receipt: resume the publish, no go-ahead needed. This does NOT
                // relax the mid-publish crash case (publish reached, no receipt),
                // which stays `Unverifiable` below.
                ResumeAction::ResumePublish
            } else if allow_unverified {
                // Publish WAS reached, so a publish could have landed pre-receipt.
                // The go-ahead accepts the double-publish risk on an unverifiable,
                // not-recorded target (adapters treat "already published" as an
                // error the coordinator then surfaces — never a silent overwrite).
                ResumeAction::ResumePublish
            } else {
                ResumeAction::Unverifiable
            }
        }
    }
}

/// Verify a target the journal never recorded a receipt for, by synthesizing a
/// receipt from the plan's coordinates and dispatching the ecosystem adapter's
/// read-only `verify` — the "did a publish land without a receipt?" question.
///
/// A target whose package the plan could not resolve cannot be queried (the caller
/// validates the plan first, so this is defensive): honest `Unknown`, never a
/// fabricated query that a registry would read as absent.
fn verify_not_recorded(
    ctx: &EffectCtx<'_>,
    pt: &PlanTarget,
    version: &str,
) -> (VerifyOutcome, Option<String>) {
    let Some(package) = pt.package.clone() else {
        return (
            VerifyOutcome::Unknown,
            Some(
                "the plan target has no resolved package name; the registry cannot be queried"
                    .to_string(),
            ),
        );
    };
    let receipt = AdapterReceipt {
        adapter: pt.adapter,
        ecosystem: pt.ecosystem,
        package,
        version: version.to_string(),
        // `verify` classifies on version + digest only; a receipt-less target has
        // no digest to compare, so presence resolves to Matches/Missing.
        canonical_ref: String::new(),
        digest: None,
        remote_url: None,
        timestamp: 0,
    };
    let outcome = resolve(pt.adapter)
        .verify(ctx, &receipt)
        .unwrap_or(VerifyOutcome::Unknown);
    (outcome, verify_reason(outcome, pt.ecosystem))
}

/// The operator-facing reason a receipt-less target verified to a non-`Matches`
/// outcome (mirrors the reconcile engine's wording so `verify` and `resume` read
/// alike).
fn verify_reason(outcome: VerifyOutcome, ecosystem: Ecosystem) -> Option<String> {
    match outcome {
        VerifyOutcome::Matches => None,
        VerifyOutcome::Missing => {
            Some("the registry does not report this version as published".to_string())
        }
        VerifyOutcome::Conflicts => {
            Some("the registry holds this version but its digest differs from the plan".to_string())
        }
        VerifyOutcome::Unknown if ecosystem == Ecosystem::Binary => Some(
            "this distribution target (GitHub Releases or a homebrew formula) is not \
             observable through the registry query"
                .to_string(),
        ),
        VerifyOutcome::Unknown => Some(
            "the registry lookup could not be performed (registry outage or unresolvable package)"
                .to_string(),
        ),
    }
}

/// The decision-level detail: what the resume will *do* about this cell, layering
/// the reconcile reason (`verify_detail`) under an action-specific explanation.
fn action_detail(
    action: ResumeAction,
    outcome: VerifyOutcome,
    journal_state: JournalState,
    publish_phase_reached: bool,
    verify_detail: Option<String>,
) -> Option<String> {
    match action {
        // Skip carries no note; Cancelled and Delegated decisions are built with
        // their own detail at the call site, so none of these reach here.
        ResumeAction::Skip | ResumeAction::Cancelled | ResumeAction::Delegated => None,
        ResumeAction::AdoptForward => Some(
            "a publish landed before its receipt was recorded; adopting it forward so it is \
             not re-published"
                .to_string(),
        ),
        ResumeAction::ResumePublish => Some(match journal_state {
            JournalState::NotRecorded if outcome == VerifyOutcome::Unknown => {
                if publish_phase_reached {
                    "unverifiable and not recorded as published; resuming the publish under the \
                     explicit go-ahead"
                        .to_string()
                } else {
                    "the publish phase was never reached, so nothing could have published; \
                     resuming the publish for this target"
                        .to_string()
                }
            }
            _ => "not published; resuming the publish for this target".to_string(),
        }),
        ResumeAction::Conflict => Some(match outcome {
            VerifyOutcome::Conflicts => {
                "a different artifact is published at this version — a human must reconcile \
                 before resuming; ossctl will not overwrite it"
                    .to_string()
            }
            VerifyOutcome::Missing => {
                "this run recorded a publish the registry no longer reports (deleted or \
                 transient) — a human must decide; ossctl will not blindly re-publish"
                    .to_string()
            }
            _ => verify_detail.unwrap_or_else(|| "conflicting registry state".to_string()),
        }),
        ResumeAction::Unverifiable => Some(verify_detail.map_or_else(
            || {
                "the reconcile could not be performed; pass --allow-unverified to proceed on trust"
                    .to_string()
            },
            |d| format!("{d}; pass --allow-unverified to proceed on trust"),
        )),
    }
}

#[cfg(test)]
mod tests;
