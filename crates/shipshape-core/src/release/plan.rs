//! The sealed, content-addressed release plan — the read-only pre-image the
//! human approves (ADR-0002 §3).
//!
//! `release plan` computes and seals a `plan_id`; `release cut --plan <plan_id>`
//! executes it and refuses on repo drift. The binary never prompts: it plans
//! and exits at the approval boundary.
//!
//! ## What `plan_id` hashes (the content address)
//!
//! [`build`] derives a [`ReleasePlan`] from the already-normalized contract and
//! detected repo facts, then content-addresses it. The `plan_id` is the
//! lowercase SHA-256 hex digest of a canonical JSON pre-image (`serde_json`,
//! whose struct-field and `BTreeMap` ordering is deterministic) covering
//! **exactly**, in this fixed order:
//!
//! 1. a domain separator + `SEAL_VERSION` — so a `plan_id` can never collide
//!    with any other shipshape digest and the canonicalization format can be
//!    evolved by a deliberate `SEAL_VERSION` bump instead of silently;
//! 2. the contract-document `schema_version` (ADR-0002 lists it explicitly);
//! 3. the **full normalized contract JSON** (`contract show`'s canonical output
//!    — every defaulted field, so any config change is drift; hashing the whole
//!    contract is deliberately *fail-closed*: a cosmetic change re-requires
//!    approval rather than risk missing a substantive one);
//! 4. the git `HEAD` sha the plan was sealed against;
//! 5. the chosen release version (the human's bump — design §3.4);
//! 6. the **resolved concrete target set** — each target's ecosystem, resolved
//!    package name, registry, and adapter *identity*. Resolution overlays
//!    facts-derived package names onto the contract's (which may be `null`), so
//!    a manifest rename is detectable drift even though the contract text is
//!    unchanged;
//! 7. the phase sequence (constant per ADR-0002 §2 for a `--bump`-less plan, so it
//!    never *causes* drift within a binary, but binding it authenticates the
//!    execution shape the approver saw and makes a future phase-model change a
//!    `SEAL_VERSION` event). A `--bump` plan prepends a `bump` phase, which this
//!    field binds;
//! 8. the engine-owned **bump plan** (`release-rust-workspace-multicrate` facet 2/3),
//!    or absent. `--bump <level>` computes a new version from the current manifest
//!    version + the level and seals the deterministic edit set (computed version,
//!    intra-workspace pin rewrites, CHANGELOG-finalize intent, any declared
//!    `bump_hook`). Omitted from the pre-image when absent (`skip_serializing_if`),
//!    so a `--bump`-less plan hashes byte-for-byte as it did before this field
//!    existed — the additive superset that made a `SEAL_VERSION` bump unnecessary.
//!
//! ## Coordinator seam (what the sibling consumes)
//!
//! The coordinator refuses a `release cut --plan <id>` on drift by re-deriving
//! current state and calling [`verify`]. It needs to persist only two plain
//! fields from an approved plan — `plan_id` and `version` — into its journal;
//! the approved [`ReleasePlan`] is otherwise reconstructed via [`build`] from
//! the journalled sealed inputs. The plan DTOs are therefore `Serialize`-only,
//! matching the repo-wide convention that the wire enums (`Ecosystem`/`Registry`
//! /`Adapter`) do not derive `Deserialize` (they collect-all-errors on parse).
//! The trust boundary is the *local journal*: an approved plan is one shipshape
//! itself wrote, not untrusted caller input.
//!
//! ## Out of this worker's scope (handed to the coordinator)
//!
//! - **Working-tree cleanliness.** The seal binds `HEAD`, not uncommitted
//!   changes. Enforcing a clean tree / executing from a clean checkout of the
//!   sealed commit is an *execution* guard the coordinator owns (it needs a new
//!   read-only `GitRepo` status port). Until then a dirty tree can publish code
//!   that differs from the sealed commit — an accepted, documented gap.
//!
//! **Adapter tool *versions* (accepted gap).** ADR-0002 §3 names "resolved
//! adapter identities+versions". The adapter registry (a sibling unit) is not
//! landed, so no adapter *tool version* (e.g. a pinned `cargo-dist` release) is
//! resolvable yet; today the address binds adapter **identity** (the enum). When
//! the registry lands, fold the resolved versions into the pre-image — a
//! deliberate `schema_version`-bumping change to what the address covers, never
//! a silent one.
//!
//! Determinism: no wall-clock, no id-gen, no ordering-unstable map enters the
//! pre-image — identical `(contract, facts, head, version)` always yield the
//! same `plan_id` (proven in tests).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::contract::schema::{ChangelogMode, Contract, Ecosystem, Registry};
use crate::protocol::facts::Facts;
use crate::protocol::plan::{
    BumpLevel, BumpPlan, ChangelogFinalizePlan, PinRewrite, PlanPhase, PlanTarget, ReleasePlan,
};

/// Build and seal a [`ReleasePlan`] from an already-normalized `contract` and
/// detected `facts`, at git `head_sha`, for the chosen `version`.
///
/// The caller (the `shipshape-cli` handler behind `release plan`, or the release
/// coordinator re-deriving current state) is responsible for having normalized
/// the contract and gathered the facts through the same code paths behind
/// `contract show` / `facts` — this function never re-parses `OSS-RELEASE.md`
/// nor re-derives facts. `version` is treated as an opaque, already-validated
/// identifier (scheme-specific validation — semver vs a calver pattern — is the
/// contract's/skill's job, not the plan's).
#[must_use]
pub fn build(contract: &Contract, facts: &Facts, head_sha: &str, version: &str) -> ReleasePlan {
    build_inner(contract, facts, head_sha, version, None)
}

/// Build and seal a `--bump` [`ReleasePlan`]: an engine-owned version-bump plan
/// that computes a new version from the current manifest version + a semantic
/// `level` and owns the deterministic edit set (`release-rust-workspace-multicrate`
/// facet 2).
///
/// `from_version` is the current `[workspace.package] version` (the tree's single
/// source of truth); the engine **computes** the new version by applying `level` to
/// it ([`crate::release::bump::bump_version`]) — the caller supplies only the level,
/// never a literal target, so the plan can never seal a `to_version` that contradicts
/// its declared `level` (the invariant lives in the core constructor, not the CLI).
/// The returned plan carries a [`PlanPhase::Bump`] at the front of its phase sequence
/// and a [`BumpPlan`] describing the edits (pin rewrites, CHANGELOG finalize, any
/// declared `bump_hook`), all folded into the content address. Its
/// [`ReleasePlan::version`] is the computed new version — every publish/tag threads it.
///
/// A `--bump`-less plan is [`build`]; the two share every non-bump derivation, so
/// the bump path is a strict additive superset.
///
/// # Errors
/// [`BumpError`](crate::release::bump::BumpError) when `from_version` is not a strict
/// `MAJOR.MINOR.PATCH` release version or the deterministic edit set contains
/// non-equivalent exact pins. The engine refuses both before sealing.
pub fn build_with_bump(
    contract: &Contract,
    facts: &Facts,
    head_sha: &str,
    from_version: &str,
    level: BumpLevel,
) -> Result<ReleasePlan, crate::release::bump::BumpError> {
    let to_version = crate::release::bump::bump_version(level, from_version)?;
    let bump = derive_bump_plan(contract, facts, head_sha, level, from_version, &to_version)?;
    Ok(build_inner(
        contract,
        facts,
        head_sha,
        &to_version,
        Some(bump),
    ))
}

/// The shared core of [`build`] / [`build_with_bump`]: resolve targets, assemble the
/// (bump-aware) phase sequence, seal, and construct the [`ReleasePlan`]. `bump` is
/// `None` for the default path (identical output and `plan_id` to before this field
/// existed) and `Some` for a `--bump` plan.
#[must_use]
fn build_inner(
    contract: &Contract,
    facts: &Facts,
    head_sha: &str,
    version: &str,
    bump: Option<BumpPlan>,
) -> ReleasePlan {
    let targets = resolve_targets(contract, facts);
    let phases = bump_aware_phases(bump.is_some());
    let plan_id = seal(
        contract,
        &targets,
        head_sha,
        version,
        &phases,
        bump.as_ref(),
    );
    ReleasePlan {
        plan_id,
        contract_schema_version: contract.schema_version,
        head_sha: head_sha.to_string(),
        version: version.to_string(),
        targets,
        phases,
        bump,
        // Carried from the (already-hashed) contract so the coordinator can hand
        // the Homebrew adapter its tap + license without re-reading the contract.
        // The first distribution that declares a tap — identical to the old
        // single-`Distribution` behavior. The release-engine CLI path
        // (`ensure_single_distribution`) rejects a multi-distribution monorepo
        // BEFORE reaching here, so `distributions.len() <= 1` and this `find_map`
        // never silently drops a second distribution's tap; carrying a per-package
        // tap for a true multi-tap monorepo is a deliberate follow-up.
        homebrew_tap: contract
            .distributions
            .iter()
            .find_map(|d| d.homebrew_tap.clone()),
        license: Some(contract.license.clone()),
        description: facts.description.clone(),
        homebrew_platforms: contract
            .distributions
            .iter()
            .flat_map(|d| d.platforms.iter().cloned())
            .collect(),
    }
}

/// Compute the content-addressed `plan_id` of a **`--bump`-less** plan for
/// `(contract, facts, head_sha, version)` **without** allocating a full
/// [`ReleasePlan`].
///
/// The drift-check seam for the coordinator: given the plan a human approved, it
/// re-derives the *current* repo's contract + facts + `HEAD`, calls this with
/// the approved plan's sealed `version`, and compares. Prefer [`verify`], which
/// wraps this and reports *which* inputs drifted; this raw form is exposed for
/// callers that only need the digest.
///
/// **No-bump only.** This seals the invariant phase sequence with **no** bump plan,
/// so it computes the id of the *no-bump* plan for these inputs — it is **not** the id
/// of a `--bump` plan (that comes from [`build_with_bump`]). The bump-aware drift check
/// lives in the CLI (`cut` re-derives via [`build_with_bump`] and compares `plan_id`
/// directly); this helper is unchanged by the bump feature and stays no-bump.
#[must_use]
pub fn compute_plan_id(
    contract: &Contract,
    facts: &Facts,
    head_sha: &str,
    version: &str,
) -> String {
    let targets = resolve_targets(contract, facts);
    seal(
        contract,
        &targets,
        head_sha,
        version,
        &PlanPhase::SEQUENCE,
        None,
    )
}

/// Check whether an `approved` plan still matches the **current** repo state.
///
/// The coordinator calls this before crossing into any irreversible phase of
/// `release cut --plan <plan_id>`. It re-derives the current `plan_id` from the
/// current `contract`, `facts`, and `head_sha`, holding the *chosen version*
/// fixed to the approved plan's (a cut may not change the sealed version — that
/// would require a new plan). `Ok(())` means the approval is still valid; a
/// [`PlanDrift`] carries the mismatched id pair and human-readable reasons for
/// the `plan_stale` error envelope. The `plan_id` mismatch is authoritative;
/// the reasons are **best-effort and may be non-exhaustive** — the approved
/// plan intentionally does not retain the old normalized contract (trust the
/// journal, not a re-supplied contract), so an exact field-level contract diff
/// is not possible here. When more than one input drifts, the reasons name
/// every one they can pinpoint (`HEAD`, schema version, target set) and fall
/// back to a generic contract-changed note only when none of those explain it.
///
/// # Errors
/// Returns [`PlanDrift`] when the recomputed `plan_id` differs from
/// `approved.plan_id` — i.e. the repo moved (a commit, a manifest rename, a
/// schema bump, a target-set change, or any normalized-contract change) since
/// approval.
pub fn verify(
    approved: &ReleasePlan,
    contract: &Contract,
    facts: &Facts,
    head_sha: &str,
) -> Result<(), PlanDrift> {
    let current_targets = resolve_targets(contract, facts);
    // Hold the sealed *shape* — the approved plan's phase sequence and bump plan —
    // fixed while re-deriving targets from the current contract/facts: verify checks
    // for contract/head/target drift, not a re-computation of the bump itself (a cut
    // recomputes the bump from `--bump` + the current manifest via `build_with_bump`
    // and compares `plan_id` directly; verify is the read-only reconcile seam).
    let current_id = seal(
        contract,
        &current_targets,
        head_sha,
        &approved.version,
        &approved.phases,
        approved.bump.as_ref(),
    );
    if current_id == approved.plan_id {
        return Ok(());
    }

    // The ids differ; pinpoint *why* so the coordinator can surface an
    // actionable `plan_stale` message rather than a bare hash mismatch.
    let mut reasons = Vec::new();
    if approved.head_sha != head_sha {
        reasons.push(format!(
            "HEAD moved from {} to {}",
            short_sha(&approved.head_sha),
            short_sha(head_sha)
        ));
    }
    if approved.contract_schema_version != contract.schema_version {
        reasons.push(format!(
            "contract schema_version changed from {} to {}",
            approved.contract_schema_version, contract.schema_version
        ));
    }
    if approved.targets != current_targets {
        reasons.push(
            "the resolved target set changed (a target, package, registry, or adapter differs)"
                .to_string(),
        );
    }
    // A change the specific probes above did not catch (any other normalized
    // contract field: version scheme, changelog, license, health badges, …).
    if reasons.is_empty() {
        reasons.push("the normalized contract changed".to_string());
    }

    Err(PlanDrift {
        approved_plan_id: approved.plan_id.clone(),
        current_plan_id: current_id,
        reasons,
    })
}

/// Why a `release cut --plan <plan_id>` was refused: the current repo no longer
/// hashes to the approved plan (ADR-0002 §3, `plan_stale`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanDrift {
    /// The `plan_id` the human approved.
    pub approved_plan_id: String,
    /// The `plan_id` the current repo state produces.
    pub current_plan_id: String,
    /// Human-readable specifics of what drifted (`HEAD` moved, the target set
    /// changed, …) — at least one entry.
    pub reasons: Vec<String>,
}

/// Whether a publish target derives its release version from a package manifest
/// the version guard can read, or has no manifest version by design — the capability
/// the fail-closed guard keys on (`version-source-fail-closed-nonrust`).
///
/// The distinction is a function of the target's **[`Ecosystem`]**, not its publish
/// registry. A Rust/Node/Python package carries its version in a manifest
/// (`Cargo.toml`/`package.json`/`pyproject.toml`) regardless of *where* it is
/// published — a Rust crate repackaged for a Homebrew tap still reads its version
/// from `Cargo.toml`, so it is [`Manifest`](VersionSource::Manifest). Keying on the
/// registry instead would wrongly treat that crate (and a binary-distribution-only
/// Rust repo) as versionless and refuse to derive a version that is plainly in the
/// tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// The ecosystem carries the package version in a manifest
    /// (`rust`←`Cargo.toml`, `node`←`package.json`, `python`←`pyproject.toml`/`setup.py`).
    /// A resolved target of this class **must** expose a detected manifest version in
    /// `facts`; a resolved package with none is a *detector failure* that fails the
    /// guard **closed** ([`VersionResolveError::MissingManifestVersion`]) rather than
    /// silently skipping the version check (the fail-OPEN gap for manifest-versioned
    /// non-Rust ecosystems this model closes).
    Manifest,
    /// No manifest version **by design**: the ecosystem's version does not live in a
    /// tree manifest — a raw `binary` distribution (its version binds to the artifact
    /// it ships), or a VCS-tag-versioned `go` module (`go.mod` declares no version).
    /// Legitimately **skipped** by the version guard: there is no manifest to read a
    /// version from and none is expected.
    Distribution,
}

impl VersionSource {
    /// Classify a target by its [`Ecosystem`] (the ecosystem is the authority on
    /// whether a package's version lives in a tree manifest).
    ///
    /// Exhaustive over [`Ecosystem`] on purpose — a new ecosystem must make a
    /// deliberate manifest-vs-distribution choice here rather than default to a silent
    /// skip (which would re-open the fail-OPEN gap).
    #[must_use]
    pub fn of(ecosystem: Ecosystem) -> Self {
        match ecosystem {
            // Ecosystems whose package version lives in a version-carrying manifest.
            Ecosystem::Rust | Ecosystem::Node | Ecosystem::Python => Self::Manifest,
            // No tree-manifest version: a raw binary (versioned by the built artifact),
            // or a Go module (versioned by its VCS tag).
            Ecosystem::Go | Ecosystem::Binary => Self::Distribution,
        }
    }
}

/// One publishable target's resolved package paired with the version its **tree
/// manifest** declares — the version the ecosystem's publish command (`cargo
/// publish` reading `Cargo.toml`, …) would **actually** upload.
///
/// The workspace manifest is the single source of truth for the release version
/// ([`resolve_release_version`]); this is one row of that truth. A tree whose
/// manifests disagree among themselves carries a set of these
/// ([`VersionResolveError::InconsistentTree`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionMismatch {
    /// The resolved package this row describes.
    pub package: String,
    /// The package's ecosystem.
    pub ecosystem: Ecosystem,
    /// The version declared in the tree manifest — what the ecosystem's publish
    /// command (`cargo publish` reading `Cargo.toml`, …) would **actually**
    /// upload for this package.
    pub manifest_version: String,
}

/// A manifest-versioned target ([`VersionSource::Manifest`]) whose resolved package
/// has **no** detected manifest version in `facts` — the fail-closed row for
/// `version-source-fail-closed-nonrust`.
///
/// Unlike a [`VersionSource::Distribution`] target (skipped by design), a manifest
/// target with no readable version means the detector failed on an ecosystem that
/// *is* manifest-versioned. The guard refuses rather than publish an unchecked
/// version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnversionedTarget {
    /// The resolved package whose manifest version could not be read.
    pub package: String,
    /// The package's ecosystem.
    pub ecosystem: Ecosystem,
    /// The publish destination — the registry whose manifest a version was expected
    /// from (`npm`←`package.json`, `PyPI`←`pyproject.toml`, …).
    pub registry: Registry,
}

/// Why a single release version could not be resolved from the workspace manifest —
/// the **single source of truth** for the release version. `shipshape release cut`
/// publishes the version already in the tree; there is no `--version` input to
/// override it (`release-drop-version-flag`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionResolveError {
    /// One or more **manifest-versioned** targets ([`VersionSource::Manifest`]) have a
    /// resolved package but **no** detected manifest version — the detector returned
    /// nothing for an ecosystem that *is* manifest-versioned (npm/PyPI/…). Failing
    /// **closed** here (rather than silently skipping the target) is the fix for
    /// `version-source-fail-closed-nonrust`: a distribution target is skipped by
    /// design, but a manifest target with no readable version is a bug that must not
    /// publish an unchecked version. Carries each such target (sorted, one per
    /// package).
    MissingManifestVersion {
        /// Every manifest-versioned target whose version could not be read.
        targets: Vec<UnversionedTarget>,
    },
    /// The tree's publishable manifests declare **more than one distinct version**,
    /// so there is no single source of truth to derive the release version from —
    /// bring the workspace into lockstep first. Carries each checkable target's
    /// package + version (sorted, one per package).
    InconsistentTree {
        /// Every checkable target and the version its manifest declares.
        versions: Vec<VersionMismatch>,
    },
    /// No manifest version could be detected — every target is a distribution target
    /// with no manifest version by design (or has no resolved package) — so there is
    /// no manifest to derive the release version from. With the `--version` input
    /// removed, the release version can **only** come from a manifest; a repo with no
    /// version-carrying manifest cannot be cut until one declares a version.
    Undeterminable,
}

/// Resolve the release version from the workspace manifest — the **single source of
/// truth**.
///
/// `shipshape release cut` does **not** bump the manifest: each ecosystem's publish
/// command uploads the version already in the tree (`cargo publish` reads
/// `Cargo.toml`), and the engine threads that version into every registry probe,
/// index-wait, and receipt. So the version a cut publishes is a **projection of the
/// tree**, not an independent input — there is no `--version` flag to override it
/// (`release-drop-version-flag`), which removes the two-masters footgun at the root
/// (a flag and the manifest could silently drift, the engine publishing the manifest
/// version while waiting for/recording the flag's, which never lands —
/// `release-cut-publish-noop`).
///
/// The manifest version is the distinct version shared by every **checkable** target
/// (a [`VersionSource::Manifest`] target with a detected manifest version in
/// `facts`). A [`VersionSource::Distribution`] target (a homebrew/binary/cargo-dist
/// target) has no manifest version by design — its release version is bound to the
/// crate it repackages — so it is skipped. A manifest-versioned target whose version
/// the detector could not read is **not** skipped: it fails the guard closed
/// (`version-source-fail-closed-nonrust`).
///
/// # Errors
/// - [`VersionResolveError::MissingManifestVersion`] — a manifest-versioned target
///   has a resolved package but no readable manifest version (fail closed).
/// - [`VersionResolveError::InconsistentTree`] — the checkable targets declare more
///   than one distinct version, so no single source of truth exists.
/// - [`VersionResolveError::Undeterminable`] — no manifest version anywhere to derive
///   from.
pub fn resolve_release_version(
    contract: &Contract,
    facts: &Facts,
) -> Result<String, VersionResolveError> {
    let classified = classify_target_versions(contract, facts);

    // Fail CLOSED first: a manifest-versioned target whose version the detector could
    // not read is NOT silently skipped (that would fail OPEN — publishing a version no
    // guard confirmed). This is the `version-source-fail-closed-nonrust` fix.
    if !classified.missing.is_empty() {
        return Err(VersionResolveError::MissingManifestVersion {
            targets: classified.missing,
        });
    }

    let distinct: BTreeSet<&str> = classified
        .checkable
        .iter()
        .map(|m| m.manifest_version.as_str())
        .collect();

    match distinct.len() {
        // No manifest version anywhere to derive from (every target is a distribution
        // target, or has no resolved package). With `--version` removed there is no
        // fallback — a repo without a version-carrying manifest cannot be cut.
        0 => Err(VersionResolveError::Undeterminable),
        // One source of truth: every checkable row shares it, so any row's version is
        // THE manifest version.
        1 => Ok(classified.checkable[0].manifest_version.clone()),
        // The tree disagrees with itself — no single source of truth to project.
        _ => Err(VersionResolveError::InconsistentTree {
            versions: classified.checkable,
        }),
    }
}

/// The version-source classification of a repo's resolved targets: the checkable
/// rows the release version is projected from, and the manifest-versioned targets
/// whose version could not be read (the fail-closed set).
struct ClassifiedVersions {
    /// [`VersionSource::Manifest`] targets **with** a detected manifest version — the
    /// checkable set the single release version is derived from.
    checkable: Vec<VersionMismatch>,
    /// [`VersionSource::Manifest`] targets with a resolved package but **no** detected
    /// manifest version — the fail-closed set (`version-source-fail-closed-nonrust`).
    missing: Vec<UnversionedTarget>,
}

/// Classify every resolved target by its [`VersionSource`], separating the checkable
/// manifest versions from the manifest-versioned targets whose version could not be
/// read.
///
/// - A [`VersionSource::Distribution`] target (a `binary`/`go` ecosystem) is skipped
///   regardless of version: it has no tree-manifest version by design.
/// - A [`VersionSource::Manifest`] target with a detected version becomes a `checkable`
///   row; one with a resolved package but **no** detected version becomes a `missing`
///   row (fail closed).
/// - A manifest target with **no resolved package** cannot be looked up here at all.
///   Package resolution is a separate concern guarded elsewhere — `release plan` warns
///   and `release cut` refuses via `coordinator::validate_plan` — so it is not
///   double-reported here as a version failure. (Deeper: hardening the resolver itself
///   to fail closed on an unresolved manifest target is tracked as a follow-up.)
fn classify_target_versions(contract: &Contract, facts: &Facts) -> ClassifiedVersions {
    let mut checkable: Vec<VersionMismatch> = Vec::new();
    let mut missing: Vec<UnversionedTarget> = Vec::new();
    // Publish-none: the contract declares NO publish target (an authored `targets: []`),
    // so there is no target to project a version through — yet such a repo is still
    // version-tracked and tagged (that is what its tag-only cut produces). Project the
    // version from the tree's own manifests instead, for the ecosystems the contract
    // declares. The rule is unchanged, only its input: one distinct manifest version is
    // the release version, several are an `InconsistentTree`, none is `Undeterminable`.
    // A package with no readable version is skipped rather than failing closed — the
    // fail-closed set exists to stop an *unchecked publish*, and here nothing is ever
    // published; a repo with no version anywhere still lands on `Undeterminable`.
    if contract.targets.is_empty() {
        // SCOPE, in order of authority: a ROOT manifest (`Cargo.toml`,
        // `package.json` — the repo's own package) outranks the workspace members
        // below it. Without that preference a normal private workspace — one service
        // crate at 0.4.0 plus a support crate at 0.1.0 — could never be tagged at all,
        // and a mixed rust+node repo would compare `Cargo.toml` against `package.json`
        // and refuse forever. With it, the members only speak when no root package
        // does (a virtual workspace), where lockstep IS the expectation.
        let candidates: Vec<&crate::protocol::facts::Package> = facts
            .packages
            .iter()
            .filter(|p| {
                contract.ecosystems.contains(&p.ecosystem)
                    && VersionSource::of(p.ecosystem) == VersionSource::Manifest
                    && p.package.is_some()
                    && p.version.is_some()
            })
            .collect();
        let roots: Vec<&crate::protocol::facts::Package> = candidates
            .iter()
            .copied()
            .filter(|p| !p.manifest.contains('/'))
            .collect();
        let scoped = if roots.is_empty() {
            &candidates
        } else {
            &roots
        };
        for package in scoped {
            if let (Some(name), Some(version)) = (&package.package, &package.version) {
                checkable.push(VersionMismatch {
                    package: name.clone(),
                    ecosystem: package.ecosystem,
                    manifest_version: version.clone(),
                });
            }
        }
        checkable.sort_by(|a, b| {
            (a.ecosystem.as_str(), &a.package).cmp(&(b.ecosystem.as_str(), &b.package))
        });
        checkable.dedup_by(|a, b| a.package == b.package && a.ecosystem == b.ecosystem);
        return ClassifiedVersions { checkable, missing };
    }
    for t in resolve_targets(contract, facts) {
        // Distribution ecosystems have no tree-manifest version by design — skip them
        // whether or not `facts` happens to carry a version for their package.
        if VersionSource::of(t.ecosystem) == VersionSource::Distribution {
            continue;
        }
        // A manifest target with no resolved package cannot be version-checked here
        // (see the null-package guards named above).
        let Some(package) = t.package else { continue };
        match facts
            .packages
            .iter()
            .find(|p| p.ecosystem == t.ecosystem && p.package.as_deref() == Some(package.as_str()))
            .and_then(|p| p.version.clone())
        {
            Some(manifest_version) => checkable.push(VersionMismatch {
                package,
                ecosystem: t.ecosystem,
                manifest_version,
            }),
            // Manifest-versioned, resolved package, but the detector read no version:
            // fail closed rather than skip (the non-Rust fail-OPEN gap).
            None => missing.push(UnversionedTarget {
                package,
                ecosystem: t.ecosystem,
                registry: t.registry,
            }),
        }
    }
    // Deterministic order, and one row per package even if a package backs several
    // targets (a crate published to crates.io AND repackaged for homebrew). Sort and
    // dedup on the SAME (ecosystem, package) key so equal keys are guaranteed adjacent
    // before the consecutive-only `dedup_by` runs.
    checkable.sort_by(|a, b| {
        (a.ecosystem.as_str(), &a.package).cmp(&(b.ecosystem.as_str(), &b.package))
    });
    checkable.dedup_by(|a, b| a.package == b.package && a.ecosystem == b.ecosystem);
    missing.sort_by(|a, b| {
        (a.ecosystem.as_str(), &a.package).cmp(&(b.ecosystem.as_str(), &b.package))
    });
    missing.dedup_by(|a, b| a.package == b.package && a.ecosystem == b.ecosystem);
    ClassifiedVersions { checkable, missing }
}

/// Overlay facts-derived package names onto the contract's target set, yielding
/// the concrete targets a cut would execute, then **expand a multi-crate Rust
/// workspace** into its full dependency-ordered publish set.
///
/// Base resolution is 1:1 with the contract's (normalizer-canonical) `targets`,
/// resolving a `null` package from facts. Then [`expand_rust_workspace_members`]
/// derives the complete crates.io publish set for a Cargo workspace from
/// [`Facts::rust_workspace`]: a downstream repo that declares only its bin crate
/// still gets its lib crate planned, lib-before-bin, so a cut never `cargo publish`es
/// a crate whose `=`-pinned workspace sibling is not yet on the index
/// (`release-rust-workspace-multicrate`). A repo that already declares every member
/// (shipshape itself) is unchanged: the derived set equals what it declared.
/// The coordinator phase sequence for a plan, prepending [`PlanPhase::Bump`] when
/// the plan owns a version bump. A `--bump`-less plan yields exactly
/// [`PlanPhase::SEQUENCE`], so its sealed `phases` (and `plan_id`) are unchanged.
fn bump_aware_phases(has_bump: bool) -> Vec<PlanPhase> {
    if has_bump {
        let mut phases = Vec::with_capacity(PlanPhase::SEQUENCE.len() + 1);
        phases.push(PlanPhase::Bump);
        phases.extend_from_slice(&PlanPhase::SEQUENCE);
        phases
    } else {
        PlanPhase::SEQUENCE.to_vec()
    }
}

/// Assemble the [`BumpPlan`] — the deterministic edit set the bump phase applies —
/// from the contract + workspace facts and the caller-computed `from`/`to` versions.
fn derive_bump_plan(
    contract: &Contract,
    facts: &Facts,
    head_sha: &str,
    level: BumpLevel,
    from_version: &str,
    to_version: &str,
) -> Result<BumpPlan, crate::release::bump::BumpError> {
    Ok(BumpPlan {
        level,
        from_version: from_version.to_string(),
        to_version: to_version.to_string(),
        pin_rewrites: derive_pin_rewrites(facts, from_version, to_version)?,
        changelog_finalize: changelog_is_finalizable(contract),
        changelog: changelog_is_finalizable(contract).then(|| ChangelogFinalizePlan {
            mode: contract.changelog.mode,
            source: contract.changelog.source,
            fragment_dir: contract.changelog.fragment_dir.clone(),
            issuectl_range: issuectl_range(contract, facts, head_sha, from_version),
        }),
        // Copied from the (already-hashed) contract so the executor need not re-read it;
        // being a copy of a hashed value it adds no new content to the address beyond
        // its presence on the bump plan.
        bump_hook: contract.release.bump_hook.clone(),
    })
}

/// Whether the bump phase finalizes the CHANGELOG (`[Unreleased]` → a dated
/// `[to_version]` section).
///
/// True for the human/fragment-authored modes (`curated`, `fragment`) whose
/// `[Unreleased]` section the engine promotes on release. False for `automated`,
/// where a release bot (release-please/changesets) owns the CHANGELOG and the engine
/// must not also rewrite it (a double-writer would clash). The concrete date is a
/// cut-time value and is deliberately not part of the plan (see [`BumpPlan::changelog_finalize`]).
///
/// An **exhaustive** match (not `!= Automated`) so a future `ChangelogMode` variant —
/// e.g. a "none"/"off" that means *no* changelog to finalize — must make a deliberate
/// choice here rather than silently defaulting to engine-finalized (which would seal a
/// bump plan that promotes a changelog that does not exist).
fn issuectl_range(
    contract: &Contract,
    facts: &Facts,
    head_sha: &str,
    from_version: &str,
) -> Option<String> {
    use crate::contract::schema::ChangelogSource;

    if contract.changelog.source != ChangelogSource::IssuectlTrailers {
        return None;
    }
    let expected = format!("v{from_version}");
    Some(if facts.tags.iter().any(|tag| tag == &expected) {
        format!("{expected}..{head_sha}")
    } else {
        // The coordinator always creates `v<version>` tags. Its absence identifies
        // the first engine release for this manifest line, where the bundled skill
        // deliberately compiles the reachable history.
        head_sha.to_string()
    })
}

fn changelog_is_finalizable(contract: &Contract) -> bool {
    match contract.changelog.mode {
        ChangelogMode::Curated | ChangelogMode::Fragment => true,
        ChangelogMode::Automated => false,
    }
}

/// Derive the intra-workspace `=`-version pin rewrites the bump applies in lockstep
/// with the workspace version.
///
/// For each workspace member (including non-published wrappers), exact pins to a
/// publishable workspace member may live either in its own dependency tables or once
/// in root `[workspace.dependencies]` and be inherited with `workspace = true`. Both
/// locations are sealed and rewritten from
/// `=<from_version>` to `=<to_version>`. Entries are emitted deterministically; the
/// set is empty for a single-crate workspace or a repo with no detected workspace graph.
///
/// **Precise, not over-broad** (`release-rust-workspace-multicrate` facet 3, llm-review):
/// a rewrite is emitted **only** when the member's manifest declares that edge's
/// requirement literally as `=<from_version>` — the exact lockstep pin — across every
/// dependency table, read from
/// [`WorkspaceMember::pin_reqs`](crate::protocol::facts::WorkspaceMember). Equivalent
/// repeated declarations form one deterministic rewrite set; a mix of exact and
/// different/path-only requirements is refused here before sealing. A caret/range/
/// `workspace = true`/independently-versioned edge with no exact lockstep declaration
/// is skipped, while a different exact requirement is refused before sealing. The
/// executor applies the same
/// equivalence rule to the sealed manifest text before replacing every match.
fn exact_pin_count(
    owner: &str,
    dependency: &str,
    requirements: &[Option<String>],
    from_pin: &str,
    from_version: &str,
) -> Result<Option<usize>, crate::release::bump::BumpError> {
    let explicit = requirements.iter().filter(|req| req.is_some()).count();
    let matching = requirements
        .iter()
        .filter(|req| req.as_deref() == Some(from_pin))
        .count();
    if matching == 0 {
        if requirements
            .iter()
            .flatten()
            .any(|requirement| requirement.trim_start().starts_with('='))
        {
            return Err(crate::release::bump::BumpError {
                version: from_version.to_string(),
                reason: format!(
                    "{owner} declares exact internal pin `{dependency}` at a version other than `{from_pin}` — refusing to leave it outside the sealed edit set"
                ),
            });
        }
        return Ok(None);
    }
    if matching != explicit {
        return Err(crate::release::bump::BumpError {
            version: from_version.to_string(),
            reason: format!(
                "{owner} declares `{dependency}` with explicit requirements that differ from `{from_pin}` — refusing to seal an ambiguous pin rewrite"
            ),
        });
    }
    Ok(Some(matching))
}

fn derive_pin_rewrites(
    facts: &Facts,
    from_version: &str,
    to_version: &str,
) -> Result<Vec<PinRewrite>, crate::release::bump::BumpError> {
    let Some(workspace) = facts.rust_workspace.as_ref() else {
        return Ok(Vec::new());
    };
    if let Some(reason) = &workspace.pin_parse_error {
        return Err(crate::release::bump::BumpError {
            version: from_version.to_string(),
            reason: format!(
                "cannot seal exact Cargo pin edits because a workspace manifest could not be parsed: {reason}"
            ),
        });
    }
    let is_member: BTreeSet<&str> = workspace
        .members
        .iter()
        .map(|m| m.package.as_str())
        .collect();
    let from_pin = format!("={from_version}");
    let mut rewrites: Vec<PinRewrite> = Vec::new();
    for owner in &workspace.pin_owners {
        for (dep, requirements) in &owner.pin_reqs {
            // Only edges to another publishable member carry an intra-workspace pin.
            if !is_member.contains(dep.as_str()) {
                continue;
            }
            // Pin discovery preserves every declaration across normal, dev, build,
            // and target-specific tables. Rewrite one sealed dependency set only when
            // every declaration is provably the same exact lockstep pin. This is the
            // same equivalence rule the cut-time rewriter enforces.
            let owner_description = format!("crate `{}`", owner.package);
            if exact_pin_count(
                &owner_description,
                dep,
                requirements,
                &from_pin,
                from_version,
            )?
            .is_none()
            {
                continue;
            }
            rewrites.push(PinRewrite {
                in_package: owner.package.clone(),
                workspace_root: false,
                dependency: dep.clone(),
                from: from_pin.clone(),
                to: format!("={to_version}"),
            });
        }
    }
    for (dep, requirements) in &workspace.workspace_pin_reqs {
        if !is_member.contains(dep.as_str()) {
            continue;
        }
        if exact_pin_count(
            "root `[workspace.dependencies]`",
            dep,
            requirements,
            &from_pin,
            from_version,
        )?
        .is_none()
        {
            continue;
        }
        rewrites.push(PinRewrite {
            in_package: "workspace".to_string(),
            workspace_root: true,
            dependency: dep.clone(),
            from: from_pin.clone(),
            to: format!("={to_version}"),
        });
    }
    rewrites.sort_unstable_by(|a, b| {
        (a.workspace_root, &a.in_package, &a.dependency).cmp(&(
            b.workspace_root,
            &b.in_package,
            &b.dependency,
        ))
    });
    // Defensive dedup: a well-formed facts graph lists each (member, dep) edge once, so
    // this is a no-op in practice; it guards against a facts parser that emitted a
    // duplicate edge producing a duplicated rewrite.
    rewrites.dedup_by(|a, b| {
        a.workspace_root == b.workspace_root
            && a.in_package == b.in_package
            && a.dependency == b.dependency
    });
    Ok(rewrites)
}

/// One engine-published crate whose release is blocked by a CI-delegated workspace
/// dependency — the phase-ordering conflict [`delegated_dependency_conflicts`] finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedDependencyConflict {
    /// The crate the ENGINE would publish in publish-all.
    pub engine_package: String,
    /// The workspace crate it depends on, whose publish is CI-delegated and therefore
    /// cannot happen until the tag — which is pushed after publish-all.
    pub delegated_package: String,
}

/// Find engine-published crates.io targets that depend on a **CI-delegated** crate in
/// the same workspace — a plan that can never complete, detected before it is cut.
///
/// The barrier order is `publish-all → tag`, and a `cargo-publish-ci` crate is
/// published by the workflow the **tag push** triggers. So if an engine-published
/// crate depends on a delegated one, publish-all reaches the dependent, the cargo
/// adapter waits for the dependency to become index-visible (it cannot be — its tag
/// has not been pushed), and the cut fails on a timeout with no ordering that could
/// ever satisfy it. Retrying does not help; only editing the contract does.
///
/// The reverse edge is fine and deliberately allowed: the engine publishes the
/// dependency in publish-all, then the tag triggers CI to publish the dependent.
///
/// Read-only and derived — it never mutates the plan and is not part of the sealed
/// pre-image. Returns an empty vec when the repo is not a multi-crate workspace, when
/// the closure touches no delegated crate, or when a target's package is unresolved
/// (an ambiguous plan is refused by its own guard, and guessing here could invent a
/// conflict that does not exist).
#[must_use]
pub fn delegated_dependency_conflicts(
    plan: &ReleasePlan,
    facts: &Facts,
) -> Vec<DelegatedDependencyConflict> {
    let Some(workspace) = facts.rust_workspace.as_ref() else {
        return Vec::new();
    };
    let delegated: BTreeSet<&str> = plan
        .targets
        .iter()
        .filter(|t| t.adapter == crate::contract::schema::Adapter::CargoPublishCi)
        .filter_map(|t| t.package.as_deref())
        .collect();
    if delegated.is_empty() {
        return Vec::new();
    }
    let deps: std::collections::BTreeMap<&str, &[String]> = workspace
        .members
        .iter()
        .map(|m| (m.package.as_str(), m.workspace_deps.as_slice()))
        .collect();

    let mut conflicts = Vec::new();
    for engine in plan.targets.iter().filter(|t| is_rust_crates_io_publish(t)) {
        let Some(root) = engine.package.as_deref() else {
            continue;
        };
        // Transitive closure over intra-workspace edges. The graph is small (workspace
        // members) and `seen` makes a cyclic/diamond graph terminate.
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut stack: Vec<&str> = deps.get(root).map(|d| collect(d)).unwrap_or_default();
        while let Some(pkg) = stack.pop() {
            if !seen.insert(pkg) {
                continue;
            }
            if delegated.contains(pkg) {
                conflicts.push(DelegatedDependencyConflict {
                    engine_package: root.to_string(),
                    delegated_package: pkg.to_string(),
                });
            }
            if let Some(next) = deps.get(pkg) {
                stack.extend(collect(next));
            }
        }
    }
    conflicts.sort_by(|a, b| {
        (&a.engine_package, &a.delegated_package).cmp(&(&b.engine_package, &b.delegated_package))
    });
    conflicts.dedup();
    conflicts
}

/// Borrow a member's dependency names as `&str`s for the closure walk.
fn collect(deps: &[String]) -> Vec<&str> {
    deps.iter().map(String::as_str).collect()
}

/// Render [`delegated_dependency_conflicts`] as operator-facing messages.
#[must_use]
pub fn delegated_dependency_messages(conflicts: &[DelegatedDependencyConflict]) -> Vec<String> {
    conflicts
        .iter()
        .map(|c| {
            format!(
                "target '{}' is published by the engine but depends on workspace crate '{}', whose                  publish is CI-delegated (adapter 'cargo-publish-ci'). The engine publishes in                  publish-all, BEFORE the tag push that triggers CI — so '{}' could never be on the                  index in time and the cut would fail waiting for it. Declare '{}' as                  'cargo-publish-ci' too (let CI publish both, in its own order), or publish '{}'                  with the engine ('cargo-publish')",
                c.engine_package,
                c.delegated_package,
                c.delegated_package,
                c.engine_package,
                c.delegated_package
            )
        })
        .collect()
}

fn resolve_targets(contract: &Contract, facts: &Facts) -> Vec<PlanTarget> {
    let base: Vec<PlanTarget> = contract
        .targets
        .iter()
        .map(|t| {
            let package = t
                .package
                .clone()
                .or_else(|| resolve_package(facts, t.ecosystem));
            PlanTarget {
                ecosystem: t.ecosystem,
                package,
                registry: t.registry,
                adapter: t.adapter,
            }
        })
        .collect();
    expand_rust_workspace_members(base, facts)
}

/// Whether a resolved target is a Rust crate published to crates.io via
/// `cargo-publish` — the target class the workspace-member derivation expands (a
/// `cargo-dist` binary distribution or a non-crates.io registry is left untouched).
fn is_rust_crates_io_publish(t: &PlanTarget) -> bool {
    t.ecosystem == Ecosystem::Rust
        && t.registry == Registry::CratesIo
        && t.adapter == crate::contract::schema::Adapter::CargoPublish
}

/// Expand the crates.io `cargo-publish` Rust targets of `base` into the
/// **dependency-ordered closure** of the declared crates (lib before bin), leaving
/// every other target in place.
///
/// The gap this closes (`release-rust-workspace-multicrate`): a two-crate workspace
/// (a lib + a bin pinning `lib = "=X"`) whose contract declares **only** the bin as a
/// target would plan a single `cargo publish <bin>` — which fails, because `lib@X` is
/// not yet on crates.io. From [`Facts::rust_workspace`] this derives the bin's
/// intra-workspace dependency closure and adds each dep as its own ordered target so
/// the coordinator publishes lib → bin (ADR-0004, one target = one publish unit; the
/// coordinator walks plan order and the adapter index-waits on each crate's own deps).
///
/// **Closure, not "every member".** The publish set is the declared Rust crates.io
/// targets plus their transitive intra-workspace dependencies — **never** an unrelated
/// publishable member the contract deliberately omitted (a not-yet-release-ready
/// crate). Publishing is irreversible, so "all publishable members" would be the wrong,
/// dangerous safety property. It is still a **strict superset of what the contract
/// declared**: every declared Rust crates.io package is a closure root (a package not
/// present as a workspace member is planned as-is, never dropped). For a repo that
/// already declares every member (shipshape itself) the closure equals the declared set,
/// so its plan is unchanged.
///
/// **Ambiguity is preserved, never expanded.** If any Rust crates.io target is
/// unresolved (`package: None` — a monorepo the facts could not disambiguate), `base`
/// is returned untouched so the downstream null-package guard/warning fires; an
/// unnamed target must never be silently turned into a workspace-wide publish.
///
/// The derived targets are spliced in at the position of the **first** Rust crates.io
/// target; the contract's other targets (cargo-dist, homebrew, a non-crates.io
/// registry) keep their relative order. Cross-ecosystem/registry order is immaterial
/// to correctness (publishes are independent per registry and the single tag is taken
/// after *all* publishes), so hoisting the crates.io block changes no behavior. When
/// there is no Rust crates.io target, or the repo is not a multi-crate workspace
/// ([`Facts::rust_workspace`] is `None`), `base` is returned unchanged — so a
/// single-crate repo and every non-Rust plan are untouched.
fn expand_rust_workspace_members(base: Vec<PlanTarget>, facts: &Facts) -> Vec<PlanTarget> {
    let Some(workspace) = facts.rust_workspace.as_ref() else {
        return base;
    };
    let first_rust = base.iter().position(is_rust_crates_io_publish);
    let Some(first_rust_idx) = first_rust else {
        return base;
    };
    // Never expand an ambiguous (unresolved-package) Rust crates.io target into a
    // workspace-wide publish: leave the plan untouched so the downstream null-package
    // guard refuses it. (`is_rust_crates_io_publish` targets only.)
    if base
        .iter()
        .filter(|t| is_rust_crates_io_publish(t))
        .any(|t| t.package.is_none())
    {
        return base;
    }
    // The declared crates.io Rust packages — the closure roots (all resolved by the
    // guard above).
    let roots: Vec<String> = base
        .iter()
        .filter(|t| is_rust_crates_io_publish(t))
        .filter_map(|t| t.package.clone())
        .collect();
    // The (uniform) registry+adapter every derived member target carries — taken from
    // the representative target so the derived crates match how the contract publishes
    // Rust (crates.io / cargo-publish, by construction of `is_rust_crates_io_publish`).
    let representative = base[first_rust_idx].clone();

    let ordered_packages = dependency_closure_order(&roots, &workspace.members);
    let derived: Vec<PlanTarget> = ordered_packages
        .into_iter()
        .map(|package| PlanTarget {
            ecosystem: Ecosystem::Rust,
            package: Some(package),
            registry: representative.registry,
            adapter: representative.adapter,
        })
        .collect();

    // Splice: derived member set at the first Rust crates.io position; all other
    // (non-Rust-crates.io) targets keep their relative order around it.
    let mut out: Vec<PlanTarget> = Vec::with_capacity(base.len() + derived.len());
    let mut spliced = false;
    for t in base {
        if is_rust_crates_io_publish(&t) {
            if !spliced {
                out.extend(derived.iter().cloned());
                spliced = true;
            }
            // Drop the original Rust crates.io target — it is represented in `derived`.
            continue;
        }
        out.push(t);
    }
    out
}

/// The dependency-ordered publish set for `roots`: the transitive intra-workspace
/// dependency closure of the declared crates, topologically ordered (a dependency
/// before its dependents).
///
/// The closure follows [`WorkspaceMember::workspace_deps`](crate::protocol::facts::WorkspaceMember)
/// edges from each root. A root that is **not** a workspace member (an explicitly
/// declared package the graph did not capture) contributes no edges but is still
/// included — the superset guarantee. Only members in the closure are ordered; an
/// unrelated publishable member the contract omitted never enters the set.
fn dependency_closure_order(
    roots: &[String],
    members: &[crate::protocol::facts::WorkspaceMember],
) -> Vec<String> {
    use std::collections::BTreeMap;
    let by_name: BTreeMap<&str, &crate::protocol::facts::WorkspaceMember> =
        members.iter().map(|m| (m.package.as_str(), m)).collect();

    // Transitive closure of `roots` over workspace_deps edges.
    let mut required: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = roots.to_vec();
    while let Some(pkg) = stack.pop() {
        if !required.insert(pkg.clone()) {
            continue;
        }
        if let Some(member) = by_name.get(pkg.as_str()) {
            for dep in &member.workspace_deps {
                if !required.contains(dep) {
                    stack.push(dep.clone());
                }
            }
        }
    }

    // Topologically order only the members inside the closure (declaration order
    // preserved as the deterministic tie-break); append any root that is not a graph
    // member (no edges to order, superset guarantee) in declared order.
    let subgraph: Vec<crate::protocol::facts::WorkspaceMember> = members
        .iter()
        .filter(|m| required.contains(&m.package))
        .cloned()
        .collect();
    let mut ordered = topo_order_members(&subgraph);
    for root in roots {
        if !ordered.iter().any(|p| p == root) {
            ordered.push(root.clone());
        }
    }
    ordered
}

/// Topologically order a workspace's publishable members so a dependency precedes
/// its dependents (lib before bin) — the publish order the coordinator walks.
///
/// Kahn's algorithm with a **deterministic** tie-break: among members whose
/// intra-workspace dependencies are all already emitted, the one earliest in
/// declaration order is chosen next, so the output is stable and reproducible (a
/// requirement of the content-addressed plan). Only edges to *other listed members*
/// gate order (an edge to a filtered-out member cannot, and does not, block).
///
/// Emission is tracked **by index**, not by package name, so two members that happen
/// to share a name (Cargo forbids this, but the graph is parsed from raw manifests)
/// are both emitted rather than one masking the other. A dependency **cycle** (which
/// Cargo itself rejects among normal/build deps, so unreachable for a valid
/// workspace) cannot be ordered; the remaining members are appended in declaration
/// order rather than dropped or looped on — the plan stays a faithful superset and the
/// cut fails later with a concrete registry error, never a planner-omitted crate.
fn topo_order_members(members: &[crate::protocol::facts::WorkspaceMember]) -> Vec<String> {
    let names: BTreeSet<&str> = members.iter().map(|m| m.package.as_str()).collect();
    // Remaining dependency count per member, counting only edges to other members.
    let mut pending: Vec<usize> = members
        .iter()
        .map(|m| {
            m.workspace_deps
                .iter()
                .filter(|d| names.contains(d.as_str()) && d.as_str() != m.package)
                .count()
        })
        .collect();
    // Emitted state per member INDEX (never by name — see the doc comment).
    let mut emitted: Vec<bool> = vec![false; members.len()];
    let mut order: Vec<String> = Vec::with_capacity(members.len());
    // Each round emits the earliest-declared member whose deps are all emitted.
    while order.len() < members.len() {
        let next = (0..members.len()).find(|&i| !emitted[i] && pending[i] == 0);
        let Some(idx) = next else {
            // A cycle blocks every remaining member: append them in declaration order
            // (deterministic) rather than loop forever or drop them.
            for i in 0..members.len() {
                if !emitted[i] {
                    emitted[i] = true;
                    order.push(members[i].package.clone());
                }
            }
            break;
        };
        emitted[idx] = true;
        order.push(members[idx].package.clone());
        // Decrement dependents that depended on the just-emitted member.
        for i in 0..members.len() {
            if !emitted[i]
                && pending[i] > 0
                && members[i]
                    .workspace_deps
                    .iter()
                    .any(|d| *d == members[idx].package)
            {
                pending[i] -= 1;
            }
        }
    }
    order
}

/// The detected package name for `ecosystem`, resolved **only when
/// unambiguous** — exactly one named manifest for that ecosystem.
///
/// `None` when no manifest named one (a virtual workspace, a binary-only repo)
/// **or** when several do (a monorepo with multiple crates of one ecosystem):
/// with no per-target manifest key in the contract, picking the first would
/// silently mis-assign the same package to every `null` target, so we leave it
/// `null` for cut-time inference instead. A monorepo should declare explicit
/// per-target `package`s in the contract; the CLI warns when this fires.
fn resolve_package(facts: &Facts, ecosystem: crate::contract::schema::Ecosystem) -> Option<String> {
    let mut named = facts
        .packages
        .iter()
        .filter(|p| p.ecosystem == ecosystem && p.package.is_some());
    let first = named.next()?;
    // More than one named candidate ⇒ ambiguous ⇒ do not guess.
    if named.next().is_some() {
        return None;
    }
    first.package.clone()
}

/// Domain separator baked into every pre-image so a `plan_id` can never be
/// confused with any other SHA-256 a Shipshape subsystem might compute over
/// similar bytes. Ends in the seal-format version for readability; the numeric
/// [`SEAL_VERSION`] is also hashed as its own field.
// COMPATIBILITY (ADR-0005 §3): changing this invalidates every stored approval.
const SEAL_DOMAIN: &str = "ossctl.release-plan";

/// Version of the sealed approval interpretation: the hashing pre-image's field set,
/// order, canonicalization, **and execution semantics**. Independent of contract or
/// wire-envelope versions. Bump this (never silently) whenever the shape changes or an
/// unchanged sealed field gains a different effect, so approvals made under distinct
/// interpretations always occupy disjoint plan-id spaces.
// v11 seals workspace-wide exact-pin discovery, including declarations owned by
// non-published members. Older plan documents remain readable for resume; fresh plans
// cannot collide with approvals whose bump edit set omitted those manifests.
const SEAL_VERSION: u32 = 11;

/// The canonical hashed pre-image (see the module docs for the exact contents).
/// A dedicated struct rather than an ad-hoc byte concatenation so the field set
/// is explicit and serde's deterministic struct-field ordering fixes the byte
/// layout.
///
/// **DO NOT REORDER these fields** — field order is part of the content address,
/// so a reorder silently changes every `plan_id`. Evolve the format via
/// [`SEAL_VERSION`] instead.
#[derive(Serialize)]
struct SealInput<'a> {
    domain: &'static str,
    seal_version: u32,
    contract_schema_version: u32,
    contract: &'a Contract,
    head_sha: &'a str,
    version: &'a str,
    targets: &'a [PlanTarget],
    phases: &'a [PlanPhase],
    /// The engine-owned bump plan, or absent. Omitted from the pre-image when `None`
    /// (`skip_serializing_if`), so a `--bump`-less plan hashes byte-for-byte as it did
    /// before this field existed — the additive superset guarantee, and why the field
    /// did not require a [`SEAL_VERSION`] bump (an absent field changes no existing
    /// pre-image). A `--bump` plan's `phases` also differ (a leading `bump`), which the
    /// already-hashed `phases` field independently binds.
    #[serde(skip_serializing_if = "Option::is_none")]
    bump: Option<&'a BumpPlan>,
}

/// Serialize the pre-image to canonical JSON and return its SHA-256 hex digest.
fn seal(
    contract: &Contract,
    targets: &[PlanTarget],
    head_sha: &str,
    version: &str,
    phases: &[PlanPhase],
    bump: Option<&BumpPlan>,
) -> String {
    sha256::hex(&seal_bytes(
        contract, targets, head_sha, version, phases, bump,
    ))
}

/// Produce the canonical seal pre-image bytes used by the internal sealing routine. The durable plan
/// store persists these exact bytes and verifies them through this one seam.
pub fn seal_bytes(
    contract: &Contract,
    targets: &[PlanTarget],
    head_sha: &str,
    version: &str,
    phases: &[PlanPhase],
    bump: Option<&BumpPlan>,
) -> Vec<u8> {
    let input = SealInput {
        domain: SEAL_DOMAIN,
        seal_version: SEAL_VERSION,
        contract_schema_version: contract.schema_version,
        contract,
        head_sha,
        version,
        targets,
        phases,
        bump,
    };
    // `to_vec` on a struct of only structs/Vecs/BTreeMaps (contract's
    // `extra_fields` is a `serde_json::Map` = `BTreeMap` without the
    // `preserve_order` feature) is deterministic — no wall-clock, no HashMap,
    // no float. It is also infallible for these concrete types; `expect` (never
    // `unwrap_or_default`, which would fail *open* by hashing an empty pre-image
    // and collide every failing plan on the empty-string digest).
    serde_json::to_vec(&input).expect("release-plan pre-image is infallible to serialize")
}

/// Hash stored canonical seal bytes through the same hashing implementation that
/// seals newly-derived plans. Kept here so storage never grows its own hash.
#[must_use]
pub fn seal_id_from_bytes(bytes: &[u8]) -> String {
    sha256::hex(bytes)
}

/// Short (first 12 hex chars) `HEAD` sha for drift messages; whole string if
/// shorter.
fn short_sha(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// A self-contained SHA-256 (FIPS 180-4) so `plan_id` needs no third-party hash
/// dependency and no edit to the workspace `Cargo.toml` (a hot file). Content
/// addressing is an integrity check over local, non-adversarial inputs, so a
/// vendored reference implementation is appropriate; correctness is pinned by
/// the RFC known-answer vectors in the module tests.
mod sha256 {
    // The canonical reference form is dense in bit-twiddling and single-letter
    // working variables; the lints below fight that idiom for no clarity gain.
    #![allow(
        clippy::unreadable_literal,
        clippy::many_single_char_names,
        clippy::needless_range_loop
    )]

    use std::fmt::Write as _;

    /// SHA-256 round constants (first 32 bits of the fractional parts of the
    /// cube roots of the first 64 primes).
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    /// Initial hash values (first 32 bits of the fractional parts of the square
    /// roots of the first 8 primes).
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    /// The lowercase 64-character SHA-256 hex digest of `data`.
    pub fn hex(data: &[u8]) -> String {
        let mut h = H0;

        // Pad: 0x80, then zeros to a 56-mod-64 boundary, then the 64-bit
        // big-endian bit length.
        let mut msg = data.to_vec();
        // FIPS 180-4 caps the message at 2^64 - 1 bits; a checked multiply turns
        // the (practically unreachable) overflow into a loud panic rather than a
        // silently wrong digest.
        let bit_len = (data.len() as u64)
            .checked_mul(8)
            .expect("SHA-256 input exceeds 2^64 bits");
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in msg.chunks_exact(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[4 * i],
                    chunk[4 * i + 1],
                    chunk[4 * i + 2],
                    chunk[4 * i + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let mut a = h[0];
            let mut b = h[1];
            let mut c = h[2];
            let mut d = h[3];
            let mut e = h[4];
            let mut f = h[5];
            let mut g = h[6];
            let mut hh = h[7];

            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }

            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }

        let mut out = String::with_capacity(64);
        for v in h {
            let _ = write!(out, "{v:08x}");
        }
        out
    }
}

#[cfg(test)]
mod tests;
