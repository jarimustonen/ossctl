//! Public wire DTOs for the release engine's per-target adapter facts (ADR-0002).
//!
//! These are the shapes an [adapter](crate::release::adapter::ReleaseAdapter)
//! produces and the coordinator journals and re-emits: the dry-run command plan,
//! the build-artifact manifest, the **publish receipt** (a durable *fact* — the
//! canonical ref/digest/URL captured at publish time, never re-derived later),
//! and the read-only **verify outcome** that drives the resume/reconcile state
//! table (ADR-0003).
//!
//! Like every other `ossctl` wire surface these ride the CLI's canonical
//! envelope — a `--json` `{schema_version, data, warnings}` document or a
//! `--output=jsonl` event stream — so they carry no document version of their
//! own; [`crate::SCHEMA_VERSION`] versions the envelope they travel in. They are
//! versioned **independently** of the internal domain types so `ossctl-core` can
//! refactor adapter internals without a wire break (ADR-0001 §2). This is a hot
//! file under the migration rule: a breaking change here bumps
//! [`crate::SCHEMA_VERSION`], never silently.
//!
//! ## The `Unknown` discipline
//!
//! [`VerifyOutcome`] mirrors the audit's tri-state presence discipline: a remote
//! reconcile that *could not be performed* (a registry outage, a package the
//! `RegistryQuery` port cannot resolve) yields [`VerifyOutcome::Unknown`], never
//! [`VerifyOutcome::Missing`]. An outage must never be read as "the release did
//! not land" — that is the one classification that would drive a dangerous
//! re-publish of an already-published version.

use serde::Serialize;

use crate::contract::schema::{Adapter, Ecosystem};

/// One external command an adapter intends to run, captured as data rather than
/// executed — the atom of a [`DryRunReport`] and the auditable record of what a
/// build/publish step shelled out to.
///
/// Rendered, never re-parsed: a caller keys off [`Self::program`] /
/// [`Self::args`], and [`Self::rendered`] is the human-readable one-liner for a
/// planning envelope or a log line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedCommand {
    /// The program to invoke (`cargo`, `npm`, `twine`, `goreleaser`, `gh`, …).
    pub program: String,
    /// The arguments passed to [`Self::program`], in order.
    pub args: Vec<String>,
}

impl PlannedCommand {
    /// Build a planned command from a program and its arguments.
    pub fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
        }
    }

    /// The command as a single shell-style line (`"cargo publish --dry-run"`),
    /// for planning envelopes and log lines. Not shell-escaped — display only.
    #[must_use]
    pub fn rendered(&self) -> String {
        if self.args.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.args.join(" "))
        }
    }
}

/// The result of an adapter's `dry_run` — the re-runnable, side-effect-free
/// preview of exactly what a real cut would do for this target.
///
/// Purely descriptive: it lists the commands that *would* run (so `release plan`
/// can seal them and a human can approve the concrete actions) plus any adapter
/// notes (e.g. "publish happens in CI via a trusted-publisher workflow, not from
/// this host"). Running a dry-run never mutates external state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRunReport {
    /// The adapter identity that produced this preview.
    pub adapter: Adapter,
    /// The commands a real cut would run for this target, in order.
    pub planned_commands: Vec<PlannedCommand>,
    /// Non-fatal adapter notes about the preview (caveats, CI-driven steps).
    pub notes: Vec<String>,
}

/// The result of an adapter's `build` — the re-runnable artifact manifest.
///
/// Names the artifacts the build produced (crate `.crate` files, wheels/sdists,
/// tarballs, release binaries) so the publish phase and the journal can refer to
/// them as facts. Re-running `build` is safe (it overwrites its own outputs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildArtifacts {
    /// The adapter identity that produced these artifacts.
    pub adapter: Adapter,
    /// Identifiers (paths or names) of the built artifacts, in stable order.
    pub artifacts: Vec<String>,
    /// Non-fatal adapter notes about the build.
    pub notes: Vec<String>,
}

/// A **publish receipt** — the durable fact captured the moment a target's
/// publish landed (ADR-0002 §1).
///
/// `publish` returns this rather than `()` precisely so the canonical
/// ref/digest/URL are *recorded*, not re-derived later: a publish that landed
/// under a drifted version must be detectable, and `verify` reconciles the
/// receipt's [`Self::version`] against what the registry actually holds. The
/// receipt is journaled as a fact and is the input to [`VerifyOutcome`]
/// classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublishReceipt {
    /// The adapter identity that performed the publish.
    pub adapter: Adapter,
    /// The ecosystem whose registry the artifact was published to — the key
    /// (with [`Self::package`]) a remote reconcile queries.
    pub ecosystem: Ecosystem,
    /// The published package/crate/module name (resolved, never `null` on a
    /// receipt — a publish that could not name its package could not publish).
    pub package: String,
    /// The version that was published — the value `verify` looks for remotely.
    pub version: String,
    /// The canonical human/tooling reference for the published artifact, e.g.
    /// `crates.io/serde@1.0.0` or `pkg:npm/@scope/name@2.3.0`.
    pub canonical_ref: String,
    /// The artifact digest (content hash) when the ecosystem exposes one at
    /// publish time; `None` when it does not (a later remote digest mismatch is
    /// then undetectable and `verify` can only confirm presence).
    pub digest: Option<String>,
    /// The public URL of the published artifact, when the ecosystem has one.
    pub remote_url: Option<String>,
    /// Publish time as whole seconds since the Unix epoch (from the injected
    /// [`Clock`](crate::ports::Clock)) — a journaled fact, not wall-clock.
    pub timestamp: u64,
}

/// The typed result of an adapter's read-only `verify` — how the published
/// receipt reconciles against what the registry currently holds (ADR-0002 §1).
///
/// This drives the resume/reconcile state table (ADR-0003): `Matches` seals the
/// target as landed, `Conflicts` and `Missing` surface a human-recoverable
/// discrepancy, and `Unknown` says the check could not be performed and must not
/// be treated as `Missing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyOutcome {
    /// The registry holds the receipt's version and (where a digest is
    /// observable) it matches — the publish is confirmed landed.
    Matches,
    /// The registry holds the receipt's version but its digest differs from the
    /// receipt's — something other than what this run published is at that
    /// version. A human must reconcile; never auto-resolved.
    Conflicts,
    /// The registry does **not** hold the receipt's version — the publish did
    /// not land (or was yanked). Distinct from [`Self::Unknown`].
    Missing,
    /// The reconcile could not be performed (registry outage, unresolvable
    /// package). Reserved for "could not check" and **never** a synonym for
    /// [`Self::Missing`] — an outage must not be read as "did not land".
    Unknown,
}

impl VerifyOutcome {
    /// The wire string for this value — the single source of truth the
    /// `Serialize` derive (`rename_all = "lowercase"`) also emits, so text and
    /// JSON never drift.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matches => "matches",
            Self::Conflicts => "conflicts",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}
