//! Resume/verify reconciliation against remote registry state (ADR-0003).
//!
//! Reconciles a journaled run against remote registry state — the remote is
//! ground truth — classifying each *published* target as one of the four
//! [`VerifyOutcome`]s. Read-only: registry targets use the injected
//! [`RegistryQuery`](crate::ports::RegistryQuery), while GitHub Releases and
//! Homebrew formulas use read-only commands through the injected runner. It never
//! mutates the repo, journal, registry, Release, or tap. Backs `ossctl release
//! verify` and the read-only half of `release resume`.
//!
//! # How a receipt is classified
//!
//! For each published target the run's journaled
//! [`PublishReceipt`](crate::protocol::journal::PublishReceipt) is dispatched
//! **through the ecosystem adapter's own [`verify`](ReleaseAdapter::verify)** —
//! the same code path a live cut uses — so the reconcile inherits every adapter's
//! observability rules for free:
//!
//! - `rust`/`node`/`python`/`go` route to a registry-observing adapter; a present
//!   version ⇒ [`VerifyOutcome::Matches`], an absent one ⇒
//!   [`VerifyOutcome::Missing`]. A digest disagreement ⇒ [`VerifyOutcome::Conflicts`]
//!   *only* where the registry exposes a remote digest to compare; the current
//!   [`RegistryQuery`](crate::ports::RegistryQuery) port lists versions only, so a
//!   present version resolves to `Matches` today and `Conflicts` becomes reachable
//!   when digest observation is wired. Which ecosystems the *CLI* can actually
//!   observe is a property of the injected port — the production
//!   `RealRegistryQuery` wires `node` first, degrading the rest to `Unknown`.
//! - GitHub Releases and Homebrew taps are observed at their destinations. A
//!   missing asset/formula is [`VerifyOutcome::Missing`]; a transport or command
//!   failure remains [`VerifyOutcome::Unknown`].
//! - a registry outage, an unresolvable/absent package name, or an ecosystem this
//!   binary does not recognize all resolve to [`VerifyOutcome::Unknown`] — the
//!   audit's tri-state discipline: a lookup that could not be performed is
//!   **never** a false [`VerifyOutcome::Missing`].

use std::collections::{BTreeMap, BTreeSet};

use crate::contract::schema::{Adapter, Ecosystem, ReleaseLayout};
use crate::protocol::journal::{PublishReceipt as JournalReceipt, RunState};
use crate::protocol::plan::{PlanTarget, ReleasePlan};
use crate::protocol::reconcile::{ReconcileReport, ReconcileSummary, TargetReconcile};
use crate::protocol::release::{PublishReceipt, VerifyOutcome};

use super::adapters::{observe_cargo_dist_github_release, resolve, EffectCtx, ReleaseAdapter};
use super::journal_target_ids;

/// Reconcile a journaled run's published targets against current registry state.
///
/// Pure with respect to the world except for read-only destination observations
/// through `ctx`; it writes nothing. Iterates the run's `published`
/// receipts in stable (target-id) order, classifies each via the ecosystem
/// adapter's [`verify`](ReleaseAdapter::verify), and rolls the outcomes up into a
/// [`ReconcileReport`]. Targets that were declared but never published carry no
/// receipt and are intentionally absent from the report (a not-yet-published
/// target is not a discrepancy; the CLI surfaces them as envelope warnings).
#[must_use]
pub fn reconcile(state: &RunState, ctx: &EffectCtx<'_>) -> ReconcileReport {
    reconcile_with_plan(state, None, ctx)
}

/// Reconcile with an authenticated stored plan when one is available. The plan
/// restores exact adapter identities and platform obligations that older journal
/// receipts did not carry; the journal-only fallback still performs the strongest
/// destination observation its durable facts allow.
#[must_use]
pub fn reconcile_with_plan(
    state: &RunState,
    plan: Option<&ReleasePlan>,
    ctx: &EffectCtx<'_>,
) -> ReconcileReport {
    let plan_targets: BTreeMap<String, &PlanTarget> = plan
        .map(|plan| {
            journal_target_ids(&plan.targets)
                .into_iter()
                .zip(&plan.targets)
                .collect()
        })
        .unwrap_or_default();
    let mut ids: BTreeSet<String> = state.published.keys().cloned().collect();
    ids.extend(state.delegated.iter().cloned());
    let mut targets = Vec::with_capacity(ids.len());
    let mut summary = ReconcileSummary::default();

    for target_id in ids {
        let planned = plan_targets.get(&target_id).copied();
        let (ecosystem, package, version, outcome, detail) =
            if let Some(receipt) = state.published.get(&target_id) {
                let (outcome, detail) = classify(ctx, receipt, planned);
                (
                    receipt.ecosystem.clone(),
                    receipt.package.clone(),
                    receipt.version.clone(),
                    outcome,
                    detail,
                )
            } else {
                classify_delegated(ctx, state, &target_id, planned, plan)
            };
        match outcome {
            VerifyOutcome::Matches => summary.matches += 1,
            VerifyOutcome::Conflicts => summary.conflicts += 1,
            VerifyOutcome::Missing => summary.missing += 1,
            VerifyOutcome::Unknown => summary.unknown += 1,
        }
        targets.push(TargetReconcile {
            target: target_id,
            ecosystem,
            package,
            version,
            outcome,
            detail,
        });
    }
    summary.reconciled = targets.len();

    ReconcileReport {
        run_id: state.run_id.clone(),
        plan_id: state.plan_id.clone(),
        run_status: state.status,
        journal_seq: state.applied_seq,
        targets,
        summary,
    }
}

/// Classify one journaled receipt, returning its outcome and a reason for every
/// non-`matches` result.
///
/// The receipt is dispatched through the ecosystem adapter's `verify`; a receipt
/// that cannot even be turned into a registry query (no package name, or an
/// unrecognized ecosystem) short-circuits to `Unknown` rather than fabricating a
/// query that could yield a false `Missing`.
fn classify(
    ctx: &EffectCtx<'_>,
    receipt: &JournalReceipt,
    planned: Option<&PlanTarget>,
) -> (VerifyOutcome, Option<String>) {
    // A receipt with no package name cannot be looked up remotely — honest
    // Unknown, never a query with an empty name that a registry reads as "absent".
    let Some(package) = receipt.package.clone() else {
        return (
            VerifyOutcome::Unknown,
            Some(
                "the receipt recorded no package name; the registry cannot be queried".to_string(),
            ),
        );
    };
    // An ecosystem this binary does not recognize (a newer journal, or a
    // distribution scheme with no adapter) cannot be reconciled.
    let Some(ecosystem) = Ecosystem::parse(&receipt.ecosystem) else {
        return (
            VerifyOutcome::Unknown,
            Some(format!(
                "unrecognized ecosystem '{}'; cannot reconcile it against a registry",
                receipt.ecosystem
            )),
        );
    };

    // Stored plans retain the exact adapter. For a pre-plan-store Homebrew receipt,
    // the durable formula URL distinguishes it from a GitHub Release receipt.
    let adapter_id = planned.map_or_else(
        || {
            if ecosystem == Ecosystem::Binary
                && receipt
                    .registry_url
                    .as_deref()
                    .is_some_and(|url| url.contains("/Formula/") || url.contains("/pull/"))
            {
                Adapter::HomebrewTap
            } else {
                ecosystem.default_adapter(ReleaseLayout::Single)
            }
        },
        |target| target.adapter,
    );
    let release_receipt = PublishReceipt {
        adapter: adapter_id,
        ecosystem,
        package,
        version: receipt.version.clone(),
        // `verify` classifies on version + digest only; the canonical ref and the
        // publish timestamp are not consulted, so a placeholder is sound here.
        canonical_ref: String::new(),
        digest: receipt.digest.clone(),
        remote_url: receipt.registry_url.clone(),
        timestamp: 0,
    };

    let outcome = match resolve(adapter_id).verify(ctx, &release_receipt) {
        Ok(outcome) => outcome,
        // The default verify never errors (an outage is already Unknown); an
        // override that shells out could — treat a genuine failure as Unknown too,
        // never a false Missing.
        Err(_) => VerifyOutcome::Unknown,
    };
    (outcome, detail_for(outcome, ecosystem, Some(adapter_id)))
}

fn classify_delegated(
    ctx: &EffectCtx<'_>,
    state: &RunState,
    target_id: &str,
    planned: Option<&PlanTarget>,
    plan: Option<&ReleasePlan>,
) -> (
    String,
    Option<String>,
    String,
    VerifyOutcome,
    Option<String>,
) {
    let version = plan.map_or_else(|| state.version.clone(), |plan| plan.version.clone());
    let adapter = planned.map(|target| target.adapter).or_else(|| {
        state
            .delegated_adapters
            .get(target_id)
            .and_then(|adapter| Adapter::parse(adapter))
    });
    let ecosystem = planned.map_or(Ecosystem::Binary, |target| target.ecosystem);
    let package = planned.and_then(|target| target.package.clone());
    let outcome = match (adapter, package.as_deref()) {
        (Some(Adapter::CargoDist), _) => observe_cargo_dist_github_release(ctx, &version),
        (Some(adapter), Some(package)) => {
            let receipt = PublishReceipt {
                adapter,
                ecosystem,
                package: package.to_string(),
                version: version.clone(),
                canonical_ref: String::new(),
                digest: None,
                remote_url: None,
                timestamp: 0,
            };
            resolve(adapter)
                .verify(ctx, &receipt)
                .unwrap_or(VerifyOutcome::Unknown)
        }
        _ => VerifyOutcome::Unknown,
    };
    (
        ecosystem.as_str().to_string(),
        package,
        version,
        outcome,
        detail_for(outcome, ecosystem, adapter),
    )
}

/// The operator-facing reason for a non-`matches` outcome (`None` for `matches`).
fn detail_for(
    outcome: VerifyOutcome,
    ecosystem: Ecosystem,
    adapter: Option<Adapter>,
) -> Option<String> {
    let is_release = adapter == Some(Adapter::CargoDist) || ecosystem == Ecosystem::Binary;
    match outcome {
        VerifyOutcome::Matches => None,
        VerifyOutcome::Missing if is_release => {
            Some("the destination does not contain the expected release artifact".to_string())
        }
        VerifyOutcome::Missing => {
            Some("the registry does not report this version as published".to_string())
        }
        VerifyOutcome::Conflicts => Some(
            "the registry holds this version but its digest differs from the recorded receipt"
                .to_string(),
        ),
        VerifyOutcome::Unknown if is_release => Some(
            "the release destination could not be observed (network or command failure)"
                .to_string(),
        ),
        VerifyOutcome::Unknown => Some(
            "the registry lookup could not be performed (registry outage or unresolvable package)"
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests;
