//! Public wire DTO for `ossctl release verify` — the read-only reconcile report
//! (ADR-0002 §1, ADR-0003 state table).
//!
//! `release verify` reads a journaled run and reconciles each published or
//! CI-delegated target against its destination. It never mutates the repo,
//! journal, registry, GitHub Release, or Homebrew tap. The result is emitted under the
//! canonical `{schema_version, data, warnings}` envelope, so it carries no
//! document version of its own ([`crate::SCHEMA_VERSION`] versions the envelope).
//!
//! ## The `Unknown` discipline
//!
//! Every per-target outcome is a [`VerifyOutcome`]; a reconcile that *could not
//! be performed* (a registry outage, an unresolvable package, or a failed
//! read-only destination query) is [`VerifyOutcome::Unknown`], **never**
//! [`VerifyOutcome::Missing`]. This follows the same
//! tri-state presence discipline `ossctl audit` uses. An outage must never be
//! read as "the release did not land".

use serde::Serialize;

use crate::protocol::journal::RunStatus;
use crate::protocol::release::VerifyOutcome;

/// The read-only reconcile report for one journaled run — the body of
/// `ossctl release verify`'s success envelope.
///
/// Reconciles the run's published and CI-delegated targets against current
/// destination state. A run that was interrupted before publishing
/// a declared target simply has no receipt to reconcile for it; that is surfaced
/// as an envelope warning by the CLI, not as a false [`VerifyOutcome::Missing`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconcileReport {
    /// The run this report reconciles.
    pub run_id: String,
    /// The sealed plan id the run executes (echoed from the journal).
    pub plan_id: String,
    /// The run's derived status at read time (`in_progress`/`completed`/
    /// `abandoned`) — context for the reconcile, which is a point-in-time snapshot
    /// of a possibly-live run.
    pub run_status: RunStatus,
    /// The high-water event sequence this report was reconciled against — the
    /// snapshot's provenance. For a live run, two reconciles taken at different
    /// `journal_seq` may legitimately differ; this pins which log prefix was seen.
    pub journal_seq: u64,
    /// One entry per published or CI-delegated target, in stable target-id order.
    pub targets: Vec<TargetReconcile>,
    /// Rollup counts across [`Self::targets`].
    pub summary: ReconcileSummary,
}

/// One published or delegated target reconciled against its destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetReconcile {
    /// The journaled target id (the `published` map key, e.g. `"cargo"`).
    pub target: String,
    /// The ecosystem the receipt was published to (`rust`, `node`, …), verbatim
    /// from the receipt.
    pub ecosystem: String,
    /// The published package/crate name, when the receipt recorded one.
    pub package: Option<String>,
    /// The version the receipt claims landed — the value reconciled remotely.
    pub version: String,
    /// How the receipt reconciles against current registry state.
    pub outcome: VerifyOutcome,
    /// A human-readable reason, present for every non-`matches` outcome (why it is
    /// `missing`/`conflicts`, or why the reconcile was `unknown`). Omitted for a
    /// clean `matches`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Rollup counts across all reconciled targets — the four [`VerifyOutcome`]
/// classes plus the total, so a caller branches on `conflicts`/`missing` without
/// re-tallying [`ReconcileReport::targets`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReconcileSummary {
    /// Total targets reconciled (`= matches + conflicts + missing + unknown`).
    pub reconciled: usize,
    /// Targets whose receipt matches registry state.
    pub matches: usize,
    /// Targets present remotely but with a differing digest. Reachable only when
    /// the registry exposes a remote digest to compare against the receipt's; the
    /// current [`RegistryQuery`](crate::ports::RegistryQuery) port lists versions
    /// only, so in production a digest-level conflict is not yet observable (a
    /// present version resolves to `matches`, never a false `conflicts`).
    pub conflicts: usize,
    /// Targets the registry does not report as published.
    pub missing: usize,
    /// Targets the reconcile could not be performed for.
    pub unknown: usize,
}
