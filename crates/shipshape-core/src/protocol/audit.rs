//! Public wire DTO for the readiness gap-report (`shipshape audit`).
//!
//! The versioned surface the `/shipshape-readiness` skill wraps: `shipshape audit` scores
//! a repo against the **gated core** (README + LICENSE + CI), the **tier-scaled
//! canon** (recommended artifacts scaled to the facts' maturity), the
//! **producer-existence** obligations the contract declares (a `fragment`
//! changelog needs its dir, a `coverage`/`scorecard` badge needs its CI
//! producer, a registry target needs an SPDX license), and the **GitHub
//! community standards** (`gh api …/community/profile`). It is **read-only** —
//! nothing here nor in [`crate::audit`] ever writes the repo (ADR-0001 §3).
//!
//! Consumers read this document under the CLI's canonical `data` envelope:
//! `{schema_version, data: <this shape>, warnings}` — the same envelope every
//! `shipshape --json` command shares (`crate::SCHEMA_VERSION` versions that wire
//! envelope). Like the facts report, the gap-report is *derived*, never
//! authored, so it has no document version of its own; the envelope's
//! `schema_version` is the single version consumers gate on.
//!
//! The report **reuses** [`Maturity`] from the canonical contract model rather
//! than restating its wire strings: the audit and the contract must agree on
//! `"mvp"` down to the byte, and sharing the one enum makes that agreement
//! structural instead of coincidental.
//!
//! ## The `unknown` discipline
//!
//! Every check distinguishes **checked-and-absent** from **could-not-check**.
//! Filesystem probes are always determinate ([`Presence::Present`] /
//! [`Presence::Absent`]). A GitHub-API or registry lookup that *fails* yields
//! [`Presence::Unknown`], never `Absent` — an outage must never be read as "the
//! artifact is missing" (issue: registry/GH-API failure ⇒ `unknown`, never
//! `false`).

use serde::Serialize;

use crate::contract::schema::Maturity;

/// Tri-state presence of one checked artifact.
///
/// `Unknown` is reserved for a check that *could not be performed* (a failed
/// `gh api` call, an unresolved remote) — it is never a synonym for `Absent`,
/// which means "checked, and the artifact is not there".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    /// The artifact was found.
    Present,
    /// The artifact was looked for and is not there.
    Absent,
    /// The check could not be completed (outage / unresolved input).
    Unknown,
}

impl Presence {
    /// The wire string for this value — the single source of truth the
    /// `Serialize` derive (`rename_all = "lowercase"`) also emits, so text and
    /// JSON never drift.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        }
    }
}

/// Whether the tier-scaled **gated core** is complete.
///
/// The gated core is README + LICENSE + CI, but the CI leg only gates at `mvp`
/// and above: a `spike` is not being published, so it is gated on README +
/// LICENSE alone and CI is reported as a (non-blocking) gap toward `mvp`
/// (design §4). `Unknown` is a forward-compatible third state; today every core
/// leg is a determinate filesystem probe, so the core resolves to
/// `Complete`/`Incomplete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoreStatus {
    /// Every applicable core artifact is present.
    Complete,
    /// At least one applicable core artifact is absent.
    Incomplete,
    /// A core leg could not be determined (never today; reserved).
    Unknown,
}

impl CoreStatus {
    /// The wire string for this value (matches the `Serialize` derive).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Unknown => "unknown",
        }
    }
}

/// Which scoring axis a gap comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    /// The gated core (README + LICENSE + CI) — the publish gate.
    Core,
    /// The tier-scaled recommended set (the "canon"): offered, never blocking.
    Canon,
    /// A producer obligation the contract declared (fragment dir, coverage /
    /// scorecard CI step, registry ⇒ SPDX license). The normalizer does **not**
    /// hard-fail on these (advisory-producer decision); the audit reports them.
    Producer,
}

impl Category {
    /// The wire string for this value (matches the `Serialize` derive).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Canon => "canon",
            Self::Producer => "producer",
        }
    }
}

/// How much a gap matters — its gating weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Part of the gated core at this tier: blocks a responsible release.
    Blocking,
    /// Tier-scaled canon or a producer obligation: offered, never blocks.
    Recommended,
}

impl Severity {
    /// The wire string for this value (matches the `Serialize` derive).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocking => "blocking",
            Self::Recommended => "recommended",
        }
    }
}

/// One unmet readiness obligation — an artifact that is absent (or could not be
/// checked) but is expected at this maturity tier (or required by the contract).
///
/// The report lists only actual gaps; a satisfied check produces no entry. Each
/// gap names the `/shipshape-*` member skill that closes it, so `/shipshape-readiness` can
/// sequence the fixes highest-severity first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gap {
    /// Stable slug for the missing artifact (`readme`, `license`, `ci`,
    /// `changelog`, `coverage`, …) — a caller keys off this, not the prose.
    pub id: String,
    /// The scoring axis this gap comes from.
    pub category: Category,
    /// The gating weight (only [`Category::Core`] gaps are ever `Blocking`).
    pub severity: Severity,
    /// Presence of the artifact — [`Presence::Absent`] or [`Presence::Unknown`]
    /// (a `Present` artifact is never a gap).
    pub status: Presence,
    /// The `/shipshape-*` member skill that closes this gap (no leading slash), e.g.
    /// `shipshape-readme`, `shipshape-ci`, `shipshape-changelog`.
    pub member: String,
    /// Human-readable explanation of what is missing and why it is expected.
    pub detail: String,
}

/// GitHub's own community-standards view of the repo — the parsed
/// `gh api repos/<owner>/<repo>/community/profile` `files` block.
///
/// Supplementary evidence alongside the filesystem-derived gaps: GitHub also
/// recognizes health files under `.github/` and `docs/`, so a `Present` here
/// can corroborate a file the root-level probe did not see. When the lookup
/// could not run ([`Self::checked`] is `false`), every field is
/// [`Presence::Unknown`] — the outage is never read as "absent".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommunityProfile {
    /// Whether the read-only `gh api …/community/profile` call succeeded. When
    /// `false`, every field below is [`Presence::Unknown`].
    pub checked: bool,
    /// A short reason when [`Self::checked`] is `false` (no GitHub remote, `gh`
    /// unavailable, API error), else `null`.
    pub unavailable_reason: Option<String>,
    /// Whether GitHub sees a README.
    pub readme: Presence,
    /// Whether GitHub sees a LICENSE.
    pub license: Presence,
    /// Whether GitHub sees a CONTRIBUTING file.
    pub contributing: Presence,
    /// Whether GitHub sees a code of conduct.
    pub code_of_conduct: Presence,
    /// Whether GitHub sees an issue template.
    pub issue_template: Presence,
    /// Whether GitHub sees a pull-request template.
    pub pull_request_template: Presence,
    /// Whether GitHub sees a SECURITY policy.
    pub security: Presence,
}

/// The readiness gap-report — a read-only score of the repo against the gated
/// core, the tier-scaled canon, the contract's producer obligations, and the
/// GitHub community standards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditReport {
    /// The canonicalized repository root that was scored.
    pub repo_root: String,
    /// The maturity tier the scoring scaled to (the contract's `maturity`).
    pub maturity: Maturity,
    /// Whether the tier-scaled gated core is complete — the publish gate the
    /// orchestrator reads to decide bootstrap-vs-cut.
    pub core_complete: CoreStatus,
    /// Every unmet obligation, in stable emit order (core first, then canon,
    /// then producer). Empty when the repo is fully ready at its tier.
    pub gaps: Vec<Gap>,
    /// GitHub's community-standards view (supplementary evidence).
    pub community_profile: CommunityProfile,
}
