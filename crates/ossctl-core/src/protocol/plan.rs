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
    /// The coordinator phase sequence a cut drives (ADR-0002 §2): dry-run-all →
    /// build-all → publish-all → tag → dist (the post-tag distribution finalize,
    /// e.g. the Homebrew formula whose tarball only exists after the tag). When the
    /// plan owns a version bump ([`Self::bump`] is `Some`), a leading
    /// [`PlanPhase::Bump`] is prepended — the engine sets the workspace version,
    /// rewrites the intra-workspace pins, refreshes the lockfile, finalizes the
    /// CHANGELOG, and runs any declared `bump_hook` **before** the crates are built,
    /// so the publish barrier builds the crates at the new version. The sequence is
    /// **part of the content address** (it authenticates the execution shape the
    /// approver saw — a plan with a bump phase can never be cut as one without);
    /// carried here so the sealed artifact is self-describing.
    pub phases: Vec<PlanPhase>,
    /// The engine-owned version-bump phase, or `null` for a plan that publishes the
    /// version already in the tree (the default, `--bump`-less path — unchanged).
    ///
    /// Present only when `release plan --bump <level>` computed a new version from
    /// the current manifest version + the semantic bump level (`release-rust-workspace-
    /// multicrate` facet 2). It carries the deterministic edit set the [`PlanPhase::Bump`]
    /// phase applies at cut time — the computed `to_version`, the intra-workspace `=`-pin
    /// rewrites, the CHANGELOG-finalize intent, and any contract-declared `bump_hook` —
    /// all folded into the content address, so approving a `--bump minor` plan and cutting
    /// it as `--bump major` (or without a bump) is drift, not a silent re-version.
    ///
    /// Omitted from the canonical JSON when `null` (`skip_serializing_if`), so a
    /// `--bump`-less plan's wire shape and `plan_id` are byte-for-byte what they were
    /// before this field existed — the `--bump` path is a strict, additive superset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bump: Option<BumpPlan>,
    /// The Homebrew tap repo (`owner/repo`) the cut's generated formula is
    /// pushed to, or `null` when the contract configured none. Copied verbatim
    /// from the (already content-addressed) normalized contract's
    /// `distribution.homebrew_tap` — carried on the plan, like [`Self::phases`],
    /// only so the coordinator can hand it to the Homebrew adapter's
    /// first-formula bootstrap without re-reading the contract. Being a copy of a
    /// value the pre-image already hashes, it changes no `plan_id`.
    pub homebrew_tap: Option<String>,
    /// The SPDX license expression the cut's generated Homebrew formula records.
    pub license: Option<String>,
    /// The package description rendered in the generated Homebrew formula.
    pub description: Option<String>,
    /// cargo-dist target triples whose release archives the Homebrew formula serves.
    pub homebrew_platforms: Vec<String>,
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

/// The engine-owned version-bump phase's deterministic edit set (`release-rust-
/// workspace-multicrate` facet 2) — the content-addressed intent the
/// [`PlanPhase::Bump`] phase applies at cut time.
///
/// The human supplies only the semantic bump [`level`](Self::level); the engine
/// **computes** [`to_version`](Self::to_version) from [`from_version`](Self::from_version)
/// (the current manifest version) + that level (major → X+1.0.0, minor → X.Y+1.0,
/// patch → X.Y.Z+1). There is no hand-typed literal version — this honours the
/// single-source-version decision (`release-drop-version-flag`): the number is
/// derived, never dictated. Every field is part of the sealed pre-image, so a
/// different bump level or a different derived edit set is drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BumpPlan {
    /// The semantic bump level the human requested (`--bump major|minor|patch`).
    pub level: BumpLevel,
    /// The current workspace version the bump was computed **from** (read from
    /// `[workspace.package] version`).
    pub from_version: String,
    /// The computed new version the bump lands **at** — set into `[workspace.package]
    /// version` and threaded into every publish/tag as the release version.
    pub to_version: String,
    /// The intra-workspace `=`-version pin rewrites the bump applies, one per
    /// (dependent-crate → pinned workspace dependency) edge whose pin tracks the
    /// bumped version (e.g. the bin's `lib = "=<from>"` → `lib = "=<to>"`). Sorted,
    /// deterministic; empty for a single-crate workspace with no intra-workspace pins.
    pub pin_rewrites: Vec<PinRewrite>,
    /// Whether the bump finalizes the CHANGELOG (`[Unreleased]` → a dated
    /// `[to_version]` section). The concrete date is a cut-time value and is
    /// deliberately **not** sealed (it would make the `plan_id` change per day); the
    /// changelog *mode* that governs the finalize is already part of the hashed
    /// contract. `false` only when the contract declares no changelog machinery.
    pub changelog_finalize: bool,
    /// The contract-declared command the engine runs in the clean checkout after the
    /// version edits (`release.bump_hook`), so version-embedding artifacts (test
    /// snapshots that embed the version) regenerate against the new version before the
    /// bump commit — `release-rust-workspace-multicrate` facet 3. `null` = no hook.
    /// Copied from the (already-hashed) contract; carried here so the executor need
    /// not re-read it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bump_hook: Option<String>,
}

/// One intra-workspace `=`-version pin the [`PlanPhase::Bump`] phase rewrites in
/// lockstep with the workspace version (e.g. the bin crate's `lib-core = "=0.1.5"`
/// → `lib-core = "=0.1.6"`). Derived deterministically from the workspace graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PinRewrite {
    /// The workspace member whose manifest carries the pin (the dependent crate).
    pub in_package: String,
    /// The pinned intra-workspace dependency crate (the pin's subject).
    pub dependency: String,
    /// The current pin requirement (`=<from_version>`).
    pub from: String,
    /// The rewritten pin requirement (`=<to_version>`).
    pub to: String,
}

/// The semantic version-bump level a human requests with `--bump` — the *only*
/// version input the engine accepts (it computes the number; the human never types
/// it). A wire enum whose kebab string is stable and part of the plan's content
/// address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BumpLevel {
    /// `X.Y.Z` → `(X+1).0.0` — a breaking change.
    Major,
    /// `X.Y.Z` → `X.(Y+1).0` — a backwards-compatible feature.
    Minor,
    /// `X.Y.Z` → `X.Y.(Z+1)` — a backwards-compatible fix.
    Patch,
}

impl BumpLevel {
    /// The wire string for this level (kebab; the single source the `Serialize`
    /// impl emits, so text and JSON never drift).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
        }
    }

    /// Parse a wire string into a level, or `None` if unrecognized (the CLI turns a
    /// `None` into the strict `--bump` value error).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "major" => Some(Self::Major),
            "minor" => Some(Self::Minor),
            "patch" => Some(Self::Patch),
            _ => None,
        }
    }

    /// Every valid wire string, for "must be one of …" messages.
    pub const VALID: &'static [&'static str] = &["major", "minor", "patch"];
}

impl serde::Serialize for BumpLevel {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

/// One phase of the coordinator's irreversibility-ordered pipeline (ADR-0002
/// §2). Every plan drives [`PlanPhase::SEQUENCE`]; a plan that owns a version
/// bump prepends [`PlanPhase::Bump`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanPhase {
    /// Engine-owned version bump (present only for a `--bump` plan): set the
    /// workspace version, rewrite the intra-workspace `=`-pins, refresh the
    /// lockfile, finalize the CHANGELOG, run any declared `bump_hook`, and commit —
    /// **before** any crate is built, so the crates build at the new version.
    Bump,
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
            Self::Bump => "bump",
            Self::DryRunAll => "dry-run-all",
            Self::BuildAll => "build-all",
            Self::PublishAll => "publish-all",
            Self::Tag => "tag",
            Self::Dist => "dist",
        }
    }

    /// The invariant phase order a `--bump`-less cut drives, dry-run-all → dist. A
    /// `--bump` plan prepends [`PlanPhase::Bump`] (see [`ReleasePlan::phases`]).
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
