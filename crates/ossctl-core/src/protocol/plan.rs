//! Public wire DTO for the sealed, content-addressed release plan
//! (`ossctl release plan` — ADR-0002 §3).
//!
//! A [`ReleasePlan`] is the read-only pre-image a human approves: the ordered
//! concrete target set a cut would publish, the invariant phase sequence the
//! coordinator will drive, the git `HEAD` it was sealed against, the chosen
//! release version, and the content-addressed [`ReleasePlan::plan_id`]. `release
//! cut --plan <plan_id>` re-derives the plan from *current* repo state and
//! refuses (`plan_stale`) if the id no longer matches — so a commit, a manifest
//! rename, a schema bump, or a different chosen version between approval and
//! execution aborts rather than silently publishing something else.
//!
//! Consumers read this document under the CLI's canonical `data` envelope:
//! `{schema_version, data: <this shape>, warnings}` — the same envelope every
//! `ossctl --json` command shares (`crate::SCHEMA_VERSION` versions that wire
//! envelope). Like `facts` and `audit`, the plan is *derived*, never authored,
//! so it has no document version of its own; the envelope's `schema_version` is
//! the wire version consumers gate on. [`ReleasePlan::contract_schema_version`]
//! is a *content* field (the contract-document version the plan was sealed
//! against, part of the content address), not the wire-envelope version.
//!
//! The plan **reuses** [`Ecosystem`], [`Registry`], and [`Adapter`] from the
//! canonical contract model rather than restating their wire strings: the plan,
//! the contract, and the release adapters must agree on `"rust"` /
//! `"crates.io"` / `"cargo-publish"` down to the byte, and sharing the one enum
//! makes that agreement structural instead of coincidental.

use serde::Serialize;

use crate::contract::schema::{Adapter, Ecosystem, Registry};

/// A sealed, content-addressed release plan — the artifact `release plan`
/// emits and a human approves.
///
/// Every field except [`Self::plan_id`] is an *input* to the content address
/// (`plan_id` is the SHA-256 digest **over** those inputs — plus the full
/// normalized contract and a domain/seal-format tag — so it is derived, never
/// authored, and is deliberately excluded from the hashed pre-image: a hash
/// cannot cover itself). See [`crate::release::plan`] for the exact pre-image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleasePlan {
    /// The content-addressed plan id: a lowercase 64-character SHA-256 hex
    /// digest over the sealed pre-image (see [`crate::release::plan`] for
    /// exactly what is hashed). Stable — identical inputs always yield this same
    /// id; any change to a hashed input yields a different one. This is the
    /// token passed back as `release cut --plan <plan_id>`.
    pub plan_id: String,
    /// The `OSS-RELEASE.md` contract-document schema version this plan was
    /// sealed against (part of the content address). A schema bump between
    /// approval and execution is drift.
    pub contract_schema_version: u32,
    /// The git `HEAD` commit sha the plan was sealed against. A new commit
    /// between approval and execution is drift.
    pub head_sha: String,
    /// The chosen release version (the human's bump per design §3.4), supplied
    /// as a validated input to `release plan`. Sealed verbatim; changing it
    /// requires a new plan.
    pub version: String,
    /// The ordered, concrete publish targets a cut would execute — one per
    /// configured `ecosystem → package → registry`, with `null` package names
    /// resolved from detected repo facts where possible.
    pub targets: Vec<PlanTarget>,
    /// The invariant coordinator phase sequence a cut drives (ADR-0002 §2):
    /// dry-run-all → build-all → publish-all → tag → dist (the post-tag
    /// distribution finalize, e.g. the Homebrew formula whose tarball only exists
    /// after the tag). Constant for every plan and therefore *not* part of the
    /// content address (an invariant cannot drift); carried here so the sealed
    /// artifact is self-describing for the approver.
    pub phases: Vec<PlanPhase>,
    /// The Homebrew tap repo (`owner/repo`) the cut's generated formula is
    /// pushed to, or `null` when the contract configured none. Copied verbatim
    /// from the (already content-addressed) normalized contract's
    /// `distribution.homebrew_tap` — carried on the plan, like [`Self::phases`],
    /// only so the coordinator can hand it to the Homebrew adapter's
    /// first-formula bootstrap without re-reading the contract. Being a copy of a
    /// value the pre-image already hashes, it changes no `plan_id`.
    pub homebrew_tap: Option<String>,
    /// The SPDX license expression the cut's generated Homebrew formula records,
    /// copied from the normalized contract's `license`. Carried for the same
    /// reason (and with the same content-address neutrality) as
    /// [`Self::homebrew_tap`].
    pub license: Option<String>,
}

/// One concrete publish destination in a sealed plan.
///
/// Mirrors the contract's [`crate::contract::schema::Target`] but is a distinct
/// wire type: the plan may *resolve* a `null` package name from repo facts, so
/// its `package` is the concrete name the human approves, not necessarily the
/// contract's (which the executor would otherwise infer at cut time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanTarget {
    /// The ecosystem this target publishes for.
    pub ecosystem: Ecosystem,
    /// The package/crate name — resolved from the detected manifest facts when
    /// the contract left it `null`; still `null` if no manifest named it (the
    /// executor would infer it at cut time).
    pub package: Option<String>,
    /// The publish destination.
    pub registry: Registry,
    /// The release tool pinned for this target.
    pub adapter: Adapter,
}

/// One phase of the coordinator's irreversibility-ordered pipeline (ADR-0002
/// §2). The sequence is invariant across every plan; [`PlanPhase::sequence`]
/// yields it in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanPhase {
    /// Dry-run every target (re-runnable, no side effects).
    DryRunAll,
    /// Build every target (re-runnable).
    BuildAll,
    /// Publish every target (per-target irreversible).
    PublishAll,
    /// Create + push the one shared git tag and GitHub Release (coordinator-only).
    Tag,
    /// Post-tag distribution finalize: targets whose artifact only exists after the
    /// tag (the Homebrew formula, whose `url` is the just-created tag archive) are
    /// finalized with the real, post-tag-computed `sha256`.
    Dist,
}

impl PlanPhase {
    /// The wire string for this phase (kebab-case; the single source the
    /// `Serialize` impl also emits, so text and JSON never drift).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DryRunAll => "dry-run-all",
            Self::BuildAll => "build-all",
            Self::PublishAll => "publish-all",
            Self::Tag => "tag",
            Self::Dist => "dist",
        }
    }

    /// The invariant phase order a cut drives, dry-run-all → dist.
    pub const SEQUENCE: [PlanPhase; 5] = [
        Self::DryRunAll,
        Self::BuildAll,
        Self::PublishAll,
        Self::Tag,
        Self::Dist,
    ];

    /// The invariant phase order a cut drives, dry-run-all → dist (borrowed view
    /// of [`Self::SEQUENCE`]; no allocation).
    #[must_use]
    pub fn sequence() -> &'static [PlanPhase] {
        &Self::SEQUENCE
    }
}

impl serde::Serialize for PlanPhase {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}
