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

/// The deterministic repo-fact report — a pure function of `(repo tree, git
/// HEAD)`, emitted by `ossctl facts` and consumed by `/oss-init` and `audit`.
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
    /// Whether a CI configuration is present (`.github/workflows` with at least
    /// one file, or a known single-file CI config).
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
    /// `>=2` recent-year committers **and** a `>=1.0` release **and** CI.
    pub production: bool,
    /// No CI **and** no `SemVer` tag **and** (single committer **or** a README
    /// self-label).
    pub spike: bool,
}
