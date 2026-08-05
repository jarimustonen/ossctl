//! Resume/verify reconciliation against remote registry state (ADR-0003).
//!
//! Reconciles a journaled run against remote registry state — the remote is
//! ground truth — classifying each *published* target as one of the four
//! [`VerifyOutcome`]s. Read-only: it queries the registry through the injected
//! [`RegistryQuery`](crate::ports::RegistryQuery) port and never mutates the repo,
//! the journal, or the registry. Backs `ossctl release verify` (and the read-only
//! half of the `release resume` reconcile decision).
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
//! - `binary` (GitHub Releases, and homebrew taps — which the contract models
//!   under the `binary` ecosystem) routes to an adapter that is **not** observable
//!   through [`RegistryQuery`](crate::ports::RegistryQuery), so it honestly reports [`VerifyOutcome::Unknown`].
//! - a registry outage, an unresolvable/absent package name, or an ecosystem this
//!   binary does not recognize all resolve to [`VerifyOutcome::Unknown`] — the
//!   audit's tri-state discipline: a lookup that could not be performed is
//!   **never** a false [`VerifyOutcome::Missing`].

use crate::contract::schema::{Ecosystem, ReleaseLayout};
use crate::protocol::journal::{PublishReceipt as JournalReceipt, RunState};
use crate::protocol::reconcile::{ReconcileReport, ReconcileSummary, TargetReconcile};
use crate::protocol::release::{PublishReceipt, VerifyOutcome};

use super::adapters::{resolve, EffectCtx, ReleaseAdapter};

/// Reconcile a journaled run's published targets against current registry state.
///
/// Pure with respect to the world *except* for the read-only registry lookups it
/// performs through `ctx` — it writes nothing. Iterates the run's `published`
/// receipts in stable (target-id) order, classifies each via the ecosystem
/// adapter's [`verify`](ReleaseAdapter::verify), and rolls the outcomes up into a
/// [`ReconcileReport`]. Targets that were declared but never published carry no
/// receipt and are intentionally absent from the report (a not-yet-published
/// target is not a discrepancy; the CLI surfaces them as envelope warnings).
#[must_use]
pub fn reconcile(state: &RunState, ctx: &EffectCtx<'_>) -> ReconcileReport {
    let mut targets = Vec::with_capacity(state.published.len());
    let mut summary = ReconcileSummary::default();

    // `published` is a BTreeMap, so iteration is already sorted by target id.
    for (target_id, receipt) in &state.published {
        let (outcome, detail) = classify(ctx, receipt);
        match outcome {
            VerifyOutcome::Matches => summary.matches += 1,
            VerifyOutcome::Conflicts => summary.conflicts += 1,
            VerifyOutcome::Missing => summary.missing += 1,
            VerifyOutcome::Unknown => summary.unknown += 1,
        }
        targets.push(TargetReconcile {
            target: target_id.clone(),
            ecosystem: receipt.ecosystem.clone(),
            package: receipt.package.clone(),
            version: receipt.version.clone(),
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
fn classify(ctx: &EffectCtx<'_>, receipt: &JournalReceipt) -> (VerifyOutcome, Option<String>) {
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

    // Dispatch through the ecosystem's adapter verify(). The specific adapter
    // identity within an ecosystem does not change verify()'s behavior (all rust
    // adapters share the registry path; all binary/homebrew adapters report
    // Unknown), so the default adapter is a faithful stand-in for the receipt,
    // which does not journal which specific adapter published it.
    let adapter_id = ecosystem.default_adapter(ReleaseLayout::Single);
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
    (outcome, detail_for(outcome, ecosystem))
}

/// The operator-facing reason for a non-`matches` outcome (`None` for `matches`).
fn detail_for(outcome: VerifyOutcome, ecosystem: Ecosystem) -> Option<String> {
    match outcome {
        VerifyOutcome::Matches => None,
        VerifyOutcome::Missing => {
            Some("the registry does not report this version as published".to_string())
        }
        VerifyOutcome::Conflicts => Some(
            "the registry holds this version but its digest differs from the recorded receipt"
                .to_string(),
        ),
        // Binary/homebrew are structurally unobservable; every other ecosystem's
        // Unknown means the lookup itself could not be performed.
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

#[cfg(test)]
mod tests;
