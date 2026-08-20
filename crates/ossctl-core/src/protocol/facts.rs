//! Public wire DTO for the deterministic repo-fact report (`ossctl facts`).
//!
//! This is the versioned surface `/oss-init` (config generation) and the
//! readiness `audit` both read, so they never disagree on maturity or the gated
//! core (ADR-0001 §3). Its field names and shape are a faithful port of
//! homebase's `infer-repo-facts.py` JSON — `ecosystems`, `packages`, `has_ci`,
//! `tags`, `committers_total`/`committers_recent_year`, `inferred_maturity`, and
//! the rest — because the prose `/oss-init` skill already relies on those exact
//! names (SCHEMA.md §4 "maturity inference signals").
//!
//! Consumers read this document under the CLI's canonical `data` envelope:
//! `{schema_version, data: <this shape>, warnings}` — the same envelope every
//! `ossctl --json` command shares (`crate::SCHEMA_VERSION` versions that wire
//! envelope). Unlike the contract document, the facts report has no
//! source-level document version of its own (it is *derived*, never authored),
//! so the envelope's `schema_version` is the single version consumers gate on.
//!
//! The report **reuses** [`Ecosystem`] and [`Maturity`] from the canonical
//! contract model rather than restating their wire strings: facts and the
//! contract must agree on `"rust"` / `"mvp"` down to the byte, and sharing the
//! one enum is how that agreement is made structural instead of coincidental.

use serde::Serialize;

use crate::contract::schema::{Ecosystem, Maturity};

/// Distribution infrastructure discovered in the repository tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DistributionSurface {
    /// Whether cargo-dist configuration is present at the repository root.
    pub has_cargo_dist: bool,
    /// Root configuration files that establish cargo-dist use.
    pub cargo_dist_evidence: Vec<String>,
    /// GitHub workflow filenames whose `push` trigger includes tags.
    pub tag_triggered_workflows: Vec<String>,
}

/// What one Cargo manifest says about publishing to crates.io.
///
/// This is deliberately tri-state: collapsing [`Self::Unknown`] into either
/// other value would turn an inconclusive read into a confident wrong statement.
/// These serialized variant names are part of the additive `facts` wire contract;
/// renaming one is a breaking change that requires a `schema_version` bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CargoPublishPolicy {
    /// The manifest permits crates.io publication: `publish` is absent or `true`,
    /// or an allow-list names `crates-io`.
    Allowed,
    /// The manifest forbids it: `publish = false`, `publish = []`, or an
    /// allow-list omits `crates-io`.
    Forbidden,
    /// The read is inconclusive, such as unresolved `publish.workspace = true`
    /// inheritance or a publish value shape the textual reader does not model.
    Unknown,
}

impl CargoPublishPolicy {
    /// The stable spelling used in JSON and human-readable output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Forbidden => "forbidden",
            Self::Unknown => "unknown",
        }
    }
}

/// The resolved crates.io publish evidence read from one Cargo manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CargoPublishFlag {
    /// Path relative to the repository root.
    pub manifest: String,
    /// The crate's `[package].name`, or `null` when it cannot be resolved.
    pub package: Option<String>,
    /// The resolved crates.io publish verdict.
    pub policy: CargoPublishPolicy,
}

/// The complete `ossctl facts --json` data payload.
///
/// [`Self::cargo_publish`] is additive to the original flat facts shape. It is
/// collected by the same detector function the contract normalizer calls for
/// its Cargo publish hard floor, preventing the inspectable report and the
/// enforcing decision from drifting apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactsReport {
    /// The established flat repo-facts fields.
    #[serde(flatten)]
    pub facts: Facts,
    /// Per-manifest Cargo publish evidence, with workspace inheritance resolved.
    pub cargo_publish: Vec<CargoPublishFlag>,
}

/// The established base repo facts, flattened into [`FactsReport`] for
/// `ossctl facts` and consumed directly by internal release and audit logic.
///
/// Every field is always present (an empty/unborn repo still gets a defined
/// value for each), mirroring the Python detector's "exit 0 even for an empty
/// repo" contract.
// The report mirrors the Python detector's flat boolean signals; grouping them
// into sub-structs purely to satisfy the bool-count lint would diverge the wire
// shape from `infer-repo-facts.py` for no consumer benefit.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Facts {
    /// The canonicalized repository root the facts were gathered from.
    pub repo_root: String,
    /// Whether the root is inside a git work tree.
    pub is_git: bool,
    /// Whether the repository has at least one commit (`HEAD` resolves).
    pub has_commits: bool,
    /// Detected packaging ecosystems, in canonical order. `[binary]` when no
    /// package manifest is found (a compiled-artifact-only repo).
    pub ecosystems: Vec<Ecosystem>,
    /// One entry per root-level package manifest that names a package (plus
    /// `Cargo.toml`/`go.mod` always, even as a virtual workspace).
    pub packages: Vec<Package>,
    /// Distinct committers across all history (`git shortlog -sne --all`).
    pub committers_total: usize,
    /// Distinct committers within the last year — a `production` signal.
    pub committers_recent_year: usize,
    /// All tag names in the repository.
    pub tags: Vec<String>,
    /// Whether any tag parses as a `SemVer` version (a release signal).
    pub has_semver_tag: bool,
    /// Whether a `>=1.0` release exists — a `>=1.0` `SemVer` tag **or** a manifest
    /// version `>=1.0`. Never probes a registry (that would need the network and
    /// break reproducibility).
    pub has_ge_1_0_release: bool,
    /// Whether a CI configuration is present (`.github/workflows` holding at
    /// least one entry, or a known single-file CI config).
    pub has_ci: bool,
    /// The detected dependency-update bot (`dependabot` / `renovate`), or `null`.
    pub dependency_bot: Option<String>,
    /// Whether an `issues/` directory is present.
    pub has_issues_dir: bool,
    /// `"spike"` when the README self-labels as pre-release/WIP, else `null`.
    pub readme_self_label: Option<String>,
    /// A short project description: the first manifest `description`, else the
    /// first non-heading README line (both truncated to 120 characters).
    pub description: Option<String>,
    /// The raw truth-table signals behind [`Self::inferred_maturity`].
    pub maturity_signals: MaturitySignals,
    /// The inferred maturity (SCHEMA.md §4 truth table, tie → `mvp`).
    pub inferred_maturity: Maturity,
    /// Detected binary-distribution infrastructure and tag-triggered workflows.
    pub distribution_surface: DistributionSurface,
    /// The Rust workspace's publishable member graph — derived plumbing the
    /// release planner needs, deliberately kept **off the JSON wire**
    /// (`#[serde(skip)]`), `None` for a repo with no multi-crate Cargo workspace.
    ///
    /// It lets [`crate::release::plan`] derive the full, dependency-ordered publish
    /// set for a multi-crate workspace: a downstream repo that declares only its bin
    /// crate as a release target still gets its lib crate planned, lib-before-bin, so
    /// `cargo publish` never hits an unindexed `=`-pinned sibling (the
    /// `release-rust-workspace-multicrate` gap). Exposing it on the wire would perturb
    /// the shared facts schema `/oss-init` and `audit` read; the plan content-addresses
    /// the resolved *targets* it produces from this graph, so the graph itself need not
    /// travel on the wire. Skipped fields still participate in [`PartialEq`], so it is a
    /// faithful part of the in-process fact value.
    #[serde(skip)]
    pub rust_workspace: Option<RustWorkspace>,
}

/// A Rust Cargo workspace's crates.io-**publishable** members and the
/// intra-workspace dependency edges among them — the graph the release planner
/// derives a dependency-ordered publish set from (`release-rust-workspace-multicrate`).
///
/// Off-wire plumbing carried on [`Facts::rust_workspace`]; see that field for why it
/// is not serialized. Members are listed in workspace declaration order (the planner
/// applies the topological ordering — a dependency before its dependents); only
/// members publishable to crates.io are included (a `publish = false` member, or one
/// restricted to a non-crates.io registry, is dropped, matching the cargo adapter's
/// cut-time `cargo metadata` filter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustWorkspace {
    /// The publishable workspace members, in declaration order.
    pub members: Vec<WorkspaceMember>,
}

/// One crates.io-publishable Cargo workspace member and its intra-workspace
/// (publishable) dependency crate names — a node in [`RustWorkspace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMember {
    /// The crate/package name (`[package].name`).
    pub package: String,
    /// The member's declared/inherited version, or `None` when unresolved.
    pub version: Option<String>,
    /// The names of this member's dependencies that are themselves publishable
    /// members of the same workspace — the edges that order the publish (each of
    /// these must be crates.io-index-visible before this member can publish). Only
    /// normal + build dependencies gate order; dev-dependencies are excluded (they
    /// never gate publish order and can legitimately cycle).
    pub workspace_deps: Vec<String>,
    /// The **literal version requirement string** this member's manifest declares
    /// for each intra-workspace dependency that carries one, keyed by dependency
    /// crate name (e.g. `{"octl-core": "=0.4.0"}` for `octl-core = { path = "…",
    /// version = "=0.4.0" }`). Only edges present in [`Self::workspace_deps`] appear,
    /// and only when the manifest declares an explicit `version` on the dependency —
    /// a path-only or `workspace = true`-inherited edge (whose requirement is not
    /// literally in this manifest) is **absent**, not defaulted.
    ///
    /// This is what makes the bump phase's pin rewrite **precise**
    /// (`release-rust-workspace-multicrate` facet 3): the planner emits a pin rewrite
    /// only for an edge whose requirement literally equals `=<from_version>` (the
    /// lockstep convention), never for a caret/range/`workspace = true` edge it would
    /// otherwise clobber. Off-wire, like the rest of [`RustWorkspace`].
    pub dep_reqs: std::collections::BTreeMap<String, String>,
    /// Every literal requirement declared for an intra-workspace dependency, across
    /// normal, dev, build, and target-specific dependency tables. `None` represents a
    /// local declaration with no literal version requirement. Unlike [`Self::dep_reqs`],
    /// this preserves duplicate declarations so release planning can prove that every
    /// pin it asks the bump executor to rewrite is equivalent.
    pub pin_reqs: std::collections::BTreeMap<String, Vec<Option<String>>>,
}

/// One detected package manifest and the name/version parsed from it.
// `package` is the field name `/oss-init` reads (SCHEMA.md §4); the
// struct-name-echo lint does not apply to a fixed wire contract.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Package {
    /// The ecosystem this manifest belongs to.
    pub ecosystem: Ecosystem,
    /// The manifest filename (e.g. `Cargo.toml`, `package.json`).
    pub manifest: String,
    /// The package/crate name, or `null` when the manifest declares none
    /// (a virtual `Cargo.toml` workspace, or a `go.mod` without a `module`).
    pub package: Option<String>,
    /// The declared version, or `null` when absent.
    pub version: Option<String>,
}

/// The two maturity truth-table outputs (SCHEMA.md §4). Both can be `false`
/// (the tie case), which resolves to `mvp`; they are never both `true`
/// (`production` is checked first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MaturitySignals {
    /// `>=2` recent-year committers **and** CI **and** a release gate. The
    /// release gate is either a `>=1.0` release **or** `ZeroVer` release evidence:
    /// a dependency-update-bot config present **and** a release cadence of `>=2`
    /// shipped (non-prerelease, `>=0.1.0`) `SemVer` tags. The second path lets a
    /// deliberately-pre-1.0 (`ZeroVer`) project reach `production` without a
    /// `>=1.0` version. The shipped-release count is not a first-class field but
    /// is recomputable from [`Facts::tags`] with the same `SemVer` parse. These
    /// are presence/name heuristics, not proofs — `/oss-init` presents them to a
    /// human for confirmation before they land in the contract.
    pub production: bool,
    /// No CI **and** no `SemVer` tag **and** (single committer **or** a README
    /// self-label).
    pub spike: bool,
}
