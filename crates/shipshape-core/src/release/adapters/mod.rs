//! Per-ecosystem release adapters behind the [`ReleaseAdapter`] trait (ADR-0002).
//!
//! One module per ecosystem — [`cargo`] (rust), [`node`], [`python`], [`go`],
//! [`homebrew`], [`binary`] — each implementing the trait's `dry_run` / `build`
//! / `publish` / `verify` steps that the [coordinator](super::coordinator)
//! drives through the phase barriers. Selection is **runtime dispatch** over an
//! **enum-backed registry** ([`EcosystemAdapter`], resolved from the contract's
//! [`Adapter`] identity by [`resolve`]); all adapters are compiled in and the
//! [`resolve`] match is exhaustive over the adapter enum, so an unwired variant
//! is a **compile error**, never a mid-release surprise. The coordinator owns
//! tagging — there is deliberately **no `tag()` method** on the trait, which is
//! what structurally enforces "tag once, after all publishes".
//!
//! ## Injected effects, per-target isolation
//!
//! Every method takes an [`EffectCtx`] (the ADR-0001 ports:
//! [`CommandRunner`],
//! [`Clock`],
//! [`RegistryQuery`]) so an adapter is unit-testable
//! against a recording fake and **never** touches the real network or process
//! table. Each adapter receives only its own [`AdapterTarget`] slice — never the
//! whole `OSS-RELEASE.md` payload — so no adapter can couple to another
//! ecosystem's config (data hiding, ADR-0002 §1).
//!
//! ## Reversibility
//!
//! `dry_run` and `build` are re-runnable and side-effect-free / self-overwriting.
//! `publish` is **per-target irreversible** — its [`PublishReceipt`] is captured
//! as a durable fact. `verify` is read-only and returns the typed
//! [`VerifyOutcome`]; a lookup that cannot be performed yields
//! [`VerifyOutcome::Unknown`], **never** a false [`VerifyOutcome::Missing`].

pub mod binary;
pub mod cargo;
pub mod go;
pub mod homebrew;
pub mod node;
pub mod python;

use std::time::Duration;

use crate::contract::schema::{Adapter, Ecosystem, Registry, Target};
use crate::ports::{Clock, CommandOutput, CommandRunner, RegistryQuery};
use crate::protocol::plan::ReleasePlan;
use crate::protocol::release::{
    BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt, VerifyOutcome,
};

/// The injected effect context every [`ReleaseAdapter`] method operates through.
///
/// Bundles the ADR-0001 ports an adapter is allowed to reach plus the repository
/// root used as the working directory for every command. Holding only trait
/// references keeps adapters testable with recording fakes and away from the real
/// network or clock.
///
/// **One scoped exception:** the [`homebrew`] adapter writes the generated `.rb`
/// directly with `std::fs` (a formula *is* a committed file; there is no "add a
/// formula" CLI to route through [`CommandRunner`]) — the first-formula *create*
/// (create-new / `O_EXCL` semantics) and the *tap-write* bump (truncating an
/// existing regular file the path first verified via `symlink_metadata`, never
/// following a symlink or creating). Both writes are confined to a private,
/// unpredictable scratch checkout the path just cloned, and they are the sole
/// direct-fs effects in the layer. A general filesystem port on this context is the
/// cleaner long-term home (tracked as issue `homebrew-adapter-fs-port`); until then
/// the exception is deliberate and local.
pub struct EffectCtx<'a> {
    /// Runs external commands (package-manager / registry CLIs). The single
    /// seam an adapter shells out through.
    pub runner: &'a dyn CommandRunner,
    /// Supplies publish timestamps for [`PublishReceipt`] as journaled facts.
    pub clock: &'a dyn Clock,
    /// Read-only registry lookups backing `verify`'s remote reconcile.
    pub registry: &'a dyn RegistryQuery,
    /// Repository root — the working directory every command runs in.
    pub repo_root: &'a std::path::Path,
    /// The concrete release artifacts threaded from build-all into publish-all
    /// (ADR-0002 §2) — the asset upload set and the source tarball a distribution
    /// adapter repackages. [`EMPTY_ARTIFACTS`] during the re-runnable
    /// dry-run / build phases (the artifacts are not yet known) and for every
    /// non-publish caller; the coordinator swaps in the computed value for the
    /// publish phase (via [`EffectCtx::with_artifacts`]) so a `publish` body can
    /// read it without re-deriving it.
    pub artifacts: &'a ReleaseArtifacts,
}

impl<'a> EffectCtx<'a> {
    /// The same effect context with `artifacts` swapped in — how the coordinator
    /// hands the computed release artifacts to the publish phase without manually
    /// re-threading every port (a new port added to [`EffectCtx`] is carried here
    /// automatically via the `..*self` update).
    #[must_use]
    pub fn with_artifacts(&self, artifacts: &'a ReleaseArtifacts) -> EffectCtx<'a> {
        EffectCtx { artifacts, ..*self }
    }

    /// The same effect context with [`repo_root`](Self::repo_root) swapped in — how
    /// the coordinator makes every adapter effect run against a **clean checkout of
    /// the sealed commit** instead of the live working tree, without `cd`-ing
    /// globally or re-threading every port. Each port reference is a `&'a`, so it is
    /// freely re-borrowed for the shorter lifetime `'b` of the (temporary) checkout
    /// path (`'a: 'b`).
    ///
    /// This is the single seam behind the reproducible-cut guarantee
    /// (`release-cut-clean-checkout`): all `dry_run` / `build` / `publish` / dist
    /// commands run in the swapped-in root, so a mid-cut edit of the operator's live
    /// tree can never change what is published. Cannot reuse the `..*self` struct
    /// update `with_artifacts` uses — that would tie the result to `'a`, but the
    /// checkout path outlives only the enclosing call.
    #[must_use]
    pub fn with_repo_root<'b>(&self, repo_root: &'b std::path::Path) -> EffectCtx<'b>
    where
        'a: 'b,
    {
        EffectCtx {
            runner: self.runner,
            clock: self.clock,
            registry: self.registry,
            repo_root,
            artifacts: self.artifacts,
        }
    }
}

/// The concrete release artifacts the coordinator threads from the build phase
/// into every adapter's [`publish`](ReleaseAdapter::publish) (ADR-0002 §2).
///
/// The two distribution adapters that repackage *already-produced* outputs need
/// inputs no single ecosystem build yields on its own:
/// [`binary`] uploads the asset paths gathered from **every**
/// target's [`build`](ReleaseAdapter::build), and [`homebrew`]'s
/// formula bump needs the published source tarball's URL + sha256. The
/// coordinator computes this once, after build-all, and exposes it through
/// [`EffectCtx::artifacts`]. The REAL registry adapters (cargo / python / go)
/// ignore it — their own CLI finds its artifacts. This is an **in-memory**
/// coordinator↔adapter hand-off only: it is never serialized or journaled, so it
/// carries no schema version of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReleaseArtifacts {
    /// Built asset/binary paths, aggregated across every target's `build` in cut
    /// order — the upload set for the binary / GitHub-Release adapter.
    pub assets: Vec<String>,
    /// The published source tarball a downstream formula bump points at, when the
    /// coordinator could resolve it (a GitHub `origin` remote). `None` when the
    /// repo has no resolvable GitHub remote.
    pub source_tarball: Option<SourceTarball>,
    /// The resolved `owner/repo` GitHub slug of the cut's `origin` remote, when
    /// the coordinator could parse one and the cut carries a GitHub-backed
    /// distribution target ([`binary`] or [`homebrew`]).
    /// The [`binary`] adapter records the GitHub-Release page URL for
    /// this slug as its receipt's [`PublishReceipt::remote_url`](crate::protocol::release::PublishReceipt::remote_url).
    /// `None` for a cut with no such target or no resolvable GitHub remote.
    pub repo_slug: Option<String>,
    /// The Homebrew formula inputs the [`homebrew`] adapter's
    /// first-formula bootstrap needs beyond the [`Self::source_tarball`] URL +
    /// sha256 — the destination tap and the SPDX license the generated `.rb`
    /// records. `None` for a cut with no homebrew target (and always `None` for
    /// every other adapter, which never reads it). Threaded from the plan by the
    /// coordinator alongside [`Self::source_tarball`].
    pub homebrew: Option<HomebrewFormula>,
    /// Verified release archives for the Homebrew formula, fetched post-tag.
    pub homebrew_assets: Vec<HomebrewAsset>,
}

/// The Homebrew-formula inputs a first-formula *create* needs that the source
/// tarball alone does not carry — the destination tap and the formula's license.
///
/// The [`homebrew`] adapter chooses its **create** vs **bump**
/// path from whether the target formula already exists in [`HomebrewFormula::tap`]; a
/// `None` tap (a `homebrew-core` target, or a `homebrew-tap` the contract left
/// unconfigured) has no bootstrap destination, so the adapter falls back to the
/// plain `bump-formula-pr` path. Like the rest of [`ReleaseArtifacts`] this is an
/// in-memory coordinator↔adapter hand-off, never serialized, so it carries no
/// schema version of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HomebrewAsset {
    /// cargo-dist target triple encoded in the archive name.
    pub triple: String,
    /// Public GitHub Release asset URL.
    pub url: String,
    /// SHA-256 of the exact archive bytes at [`Self::url`].
    pub sha256: String,
}

/// Inputs that describe a generated Homebrew formula.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HomebrewFormula {
    /// The destination tap repo as an `owner/repo` slug (from the contract's
    /// `distribution.homebrew_tap`), or `None` when the contract configured none.
    pub tap: Option<String>,
    /// The SPDX license expression the generated formula's `license` stanza
    /// records, or `None` to omit the stanza.
    pub license: Option<String>,
    /// Package metadata description, surfaced by `brew search`.
    pub description: Option<String>,
    /// Version sealed into the release plan, rendered explicitly so post-cut
    /// verification can distinguish a stale tap from a current one.
    pub version: String,
    /// Release archive triples cargo-dist publishes for this package.
    pub platforms: Vec<String>,
}

/// A shared empty artifact set — the value carried through the dry-run / build
/// phases and by every non-publish caller ([`EffectCtx::artifacts`] must always
/// point at *something*).
///
/// A module-level `static` (not an associated `const`) so `&EMPTY_ARTIFACTS` is a
/// genuine `&'static ReleaseArtifacts` that a returning function can hand out; a
/// `const` holding a `Vec` (which has `Drop`) is inlined at each use site as a
/// local temporary and cannot escape its enclosing expression.
pub static EMPTY_ARTIFACTS: ReleaseArtifacts = ReleaseArtifacts {
    assets: Vec::new(),
    source_tarball: None,
    repo_slug: None,
    homebrew: None,
    homebrew_assets: Vec::new(),
};

/// Build the plan-derived artifact context needed by read-only destination
/// verification. No build outputs are invented: only the sealed Homebrew formula
/// obligations are carried so a post-hoc verify checks the exact platform set.
#[must_use]
pub fn verification_artifacts(plan: &ReleasePlan) -> ReleaseArtifacts {
    let homebrew = plan
        .targets
        .iter()
        .any(|target| matches!(target.registry, Registry::Homebrew))
        .then(|| HomebrewFormula {
            tap: plan.homebrew_tap.clone(),
            license: plan.license.clone(),
            description: plan.description.clone(),
            version: plan.version.clone(),
            platforms: plan.homebrew_platforms.clone(),
        });
    ReleaseArtifacts {
        homebrew,
        ..ReleaseArtifacts::default()
    }
}

/// Observe the GitHub Release asset set for a delegated or engine-owned binary
/// target. Kept at the adapter seam so live cuts and post-hoc verification share
/// one parser and one outcome discipline.
#[must_use]
pub fn observe_github_release_assets(
    ctx: &EffectCtx<'_>,
    version: &str,
    expected_assets: &[String],
) -> VerifyOutcome {
    binary::observe_release_assets(ctx, version, expected_assets)
}

/// Observe cargo-dist's manifest inventory on the tagged GitHub Release.
#[must_use]
pub fn observe_cargo_dist_github_release(
    ctx: &EffectCtx<'_>,
    version: &str,
    package: &str,
) -> VerifyOutcome {
    binary::observe_cargo_dist_release(ctx, version, package)
}

/// The published source tarball a Homebrew formula bump consumes (`--url` /
/// `--sha256`).
///
/// The `url` is the deterministic GitHub source-archive URL for the cut's tag.
/// The `sha256` is `None` during the **pre-tag** phases (dry-run / build preview):
/// the tag archive the `url` points at is created only in the tag-once phase,
/// *after* publish-all (ADR-0002 §2), so it cannot be fetched-and-hashed yet, and a
/// local `git archive` is not byte-equal to GitHub's served tarball — a wrong
/// `--sha256` is worse than none. The **post-tag** dist phase then fetches the
/// pushed archive, hashes it, and threads the real `Some(sha256)` into the homebrew
/// publish so the finalized formula carries a correct hash (no draft placeholder).
/// See [`super::coordinator`]'s `source_tarball` / `dist_phase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTarball {
    /// The source tarball's public URL (the GitHub tag archive).
    pub url: String,
    /// The tarball's sha256. `None` in the pre-tag preview phases (`brew` would
    /// derive it from [`Self::url`]); `Some(<64-hex>)`, computed from the pushed tag
    /// archive, in the post-tag dist phase that finalizes the formula.
    pub sha256: Option<String>,
}

/// The per-target release input an adapter operates on: exactly one contract
/// [`Target`] slice enriched with the plan's chosen version and the resolved
/// package name.
///
/// The [`Target`] is the adapter's slice of the normalized contract; `version`
/// and `package` are resolved once by the plan/coordinator (the chosen `SemVer`
/// bump is a sealed plan input, ADR-0002 §3) and passed in, so the adapter never
/// re-derives them and never sees another target's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterTarget {
    /// The contract target this cut publishes (ecosystem, registry, adapter).
    pub target: Target,
    /// The resolved package/crate/module name (contract `package`, or the name
    /// inferred from the manifest by the plan when the contract left it `null`).
    pub package: String,
    /// The version this cut publishes — the plan's chosen bump.
    pub version: String,
}

impl AdapterTarget {
    /// The ecosystem this target publishes for.
    #[must_use]
    pub fn ecosystem(&self) -> Ecosystem {
        self.target.ecosystem
    }

    /// The canonical `registry/package@version` reference for this target — the
    /// receipt's [`PublishReceipt::canonical_ref`] and a stable log key.
    #[must_use]
    pub fn canonical_ref(&self) -> String {
        format!(
            "{}/{}@{}",
            self.target.registry.as_str(),
            self.package,
            self.version
        )
    }
}

/// Why an adapter step failed. Distinct from a *verify* discrepancy, which is a
/// successful read modelled by [`VerifyOutcome`] rather than an error.
#[derive(Debug)]
pub enum AdapterError {
    /// A command exited non-zero (or was signalled). Carries the rendered
    /// command, its exit code (`None` on signal), and captured stderr.
    Command {
        /// The command that failed, rendered as a shell-style line.
        command: String,
        /// The process exit code, or `None` if terminated by a signal.
        code: Option<i32>,
        /// Captured standard error, for the operator-facing message.
        stderr: String,
    },
    /// A command could not be spawned at all (the port returned an I/O error).
    Io {
        /// The command whose spawn failed, rendered as a shell-style line.
        command: String,
        /// The underlying I/O error rendered as text.
        source: String,
    },
    /// A local filesystem write an adapter performs *between* commands failed —
    /// distinct from [`Self::Io`] (a process that could not be spawned). The
    /// [`homebrew`] first-formula create writes the generated
    /// `.rb` into the tap checkout between the clone and the commit; a failure
    /// there is this.
    Filesystem {
        /// The path the write targeted, for the operator-facing message.
        path: String,
        /// The underlying I/O error rendered as text.
        source: String,
    },
    /// The adapter has no real implementation of this operation from this host
    /// (e.g. a CI-only trusted-publisher publish). Named so the coordinator can
    /// surface a precise, honest message rather than a fabricated receipt.
    Unsupported {
        /// The adapter identity.
        adapter: Adapter,
        /// The operation that is unsupported (`"publish"`, `"build"`, …).
        operation: &'static str,
    },
    /// A just-published artifact did not become visible on its registry index
    /// within the wait ceiling, so a dependent artifact could not be published
    /// safely. Distinct from [`Self::Command`]: the publish itself *succeeded* —
    /// only the between-publishes index-wait timed out (the multi-crate cargo
    /// workspace path, where a dependent crate must not publish until its
    /// workspace dependency is index-visible; see [`cargo`]).
    IndexTimeout {
        /// The published package still absent from the index.
        package: String,
        /// The version being waited for.
        version: String,
        /// How long the wait lasted before giving up, in seconds.
        waited_secs: u64,
    },
    /// The registry could not be reached to determine a crate's published state,
    /// so the publish path cannot make a safe decision and fails **closed** rather
    /// than guess. Raised in two places (see [`cargo`]): the pre-publish
    /// idempotency probe (cannot *prove* the crate has not already landed, so a
    /// duplicate irreversible upload is refused), and the dependency index-wait
    /// (every poll for a workspace dependency failed, so its visibility is unknown
    /// — surfaced honestly instead of a misleading [`Self::IndexTimeout`]). Mirrors
    /// the reconcile layer's outage ⇒ [`VerifyOutcome::Unknown`] discipline: an
    /// unknown remote state is never read as "safe to (re)publish".
    RegistryUnavailable {
        /// The package whose registry state could not be determined.
        package: String,
        /// The version being probed or waited for.
        version: String,
        /// The underlying registry lookup error, rendered as text.
        source: String,
    },
    /// A target's own `cargo publish` exited successfully, but the published
    /// `{package}@{version}` was **not confirmable** on the registry index within
    /// the wait ceiling — the registry answered but never showed the version. The
    /// upload *may* have landed (a slow index) or may have shipped nothing (a silent
    /// no-op: a registry-alias/credential/env difference, or an under-declared
    /// target). Either way the cut fails **closed** here rather than journal a
    /// [`PublishReceipt`] for a publish it cannot confirm (the
    /// `cut-noop-self-visibility-check` / issuectl 0.8.1 signature) — the operator
    /// resumes/verifies once the index catches up, or investigates a genuine no-op.
    /// Distinct from [`Self::IndexTimeout`] (a *dependency* a *dependent* was waiting
    /// on) and from [`Self::RegistryUnavailable`] (the registry was never reachable —
    /// an outage): this is the *self*-visibility confirm of the crate the adapter
    /// just published, the registry reachable but the version observed *absent*.
    PublishNotVisible {
        /// The package whose own publish did not become index-visible.
        package: String,
        /// The version that was published but never appeared.
        version: String,
        /// How long the confirm waited before giving up, in seconds.
        waited_secs: u64,
    },
    /// The resume idempotency skip was **refused**: `package@version` is already on
    /// the registry, but the artifact this cut would upload is **not** byte-identical
    /// to the crate already published there — the registry holds a *different*
    /// artifact at this version than this cut intended.
    ///
    /// The pre-fix skip trusted name + version existence alone and journaled a receipt
    /// without re-uploading (the last "receipt without a fresh upload" path, the same
    /// shape as the `cut-noop-self-visibility-check` no-op). This variant is the
    /// digest-authenticated refusal: the cut fails **closed** rather than skip and
    /// fabricate a receipt for a crate it did not put there. A benign cause is the
    /// version having been packaged by a different toolchain (a non-reproducible
    /// `.crate`); a malign one is a supply-chain substitution — either way the operator
    /// must investigate before the cut can proceed. Distinct from
    /// [`Self::RegistryUnavailable`] (the digest could not be read at all — an outage):
    /// here the registry answered with a concrete, *conflicting* digest.
    DigestMismatch {
        /// The already-published package whose registry artifact did not match.
        package: String,
        /// The version whose on-registry crate conflicts with the intended one.
        version: String,
        /// The sha256 (lowercase hex) of the `.crate` this cut would upload.
        local: String,
        /// The registry-recorded checksum (crates.io sparse-index `cksum`).
        remote: String,
    },
    /// A **CI-delegated** publish was planned for a version that is ALREADY on the
    /// registry, so the cut's post-tag observation could not distinguish CI's publish
    /// from the pre-existing one.
    ///
    /// Raised by the [`cargo`] adapter's `cargo-publish-ci` dry-run (pre-tag, before
    /// any irreversible step). An engine-owned publish cannot hit this — its publish
    /// path probes and digest-authenticates the registry before uploading — but a
    /// delegated target is skipped in publish-all, so `verify`'s presence check is the
    /// only gate, and presence is satisfied by an upload that predates the cut. Cutting
    /// an already-published version would therefore end GREEN while CI's `cargo
    /// publish` failed with "crate version already uploaded": a silent false green,
    /// which is the one outcome the verify barrier exists to prevent.
    DelegatedVersionAlreadyPublished {
        /// The package whose sealed version is already on the registry.
        package: String,
        /// The version that is already published.
        version: String,
    },
    /// An adapter was handed a target whose declared [`registry`](Target::registry)
    /// it does not support, so it refuses the target **before any external action**
    /// rather than risk publishing to an unexpected destination. Raised by the
    /// [`cargo`] adapter for any rust target whose registry is not
    /// [`Registry::CratesIo`] (the only rust registry shipshape supports today): cargo
    /// honors ambient registry config, so an unpinned publish could land on the
    /// wrong registry while the engine probes crates.io and records a crates.io
    /// receipt. A typed error keeps this a fail-fast misconfiguration, distinct from
    /// a command failure ([`Self::Command`]).
    UnsupportedRegistry {
        /// The adapter identity that rejected the target.
        adapter: Adapter,
        /// The declared registry that this adapter does not support.
        registry: Registry,
    },
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Command {
                command,
                code,
                stderr,
            } => {
                let code = code.map_or_else(|| "signal".to_string(), |c| c.to_string());
                write!(f, "`{command}` failed (exit {code}): {}", stderr.trim())
            }
            Self::Io { command, source } => write!(f, "cannot run `{command}`: {source}"),
            Self::Filesystem { path, source } => {
                write!(f, "cannot write `{path}`: {source}")
            }
            Self::Unsupported { adapter, operation } => write!(
                f,
                "adapter `{}` does not support `{operation}` from this host",
                adapter.as_str()
            ),
            Self::IndexTimeout {
                package,
                version,
                waited_secs,
            } => write!(
                f,
                "`{package}@{version}` was not visible on the registry index within \
                 {waited_secs}s; a crate that depends on it cannot be published until it is. \
                 If `{package}` is a workspace crate, ensure it is declared as its own release \
                 target and that its publish succeeded"
            ),
            Self::RegistryUnavailable {
                package,
                version,
                source,
            } => write!(
                f,
                "cannot reach the registry to determine the published state of \
                 `{package}@{version}` (registry unreachable: {source}); failing closed rather \
                 than risk an unsafe publish decision"
            ),
            Self::PublishNotVisible {
                package,
                version,
                waited_secs,
            } => write!(
                f,
                "`cargo publish` of `{package}@{version}` exited successfully, but the version was \
                 not visible on the crates.io index within {waited_secs}s. The upload MAY have \
                 landed (a slow index) — run `shipshape release verify`/`resume` once the index \
                 catches up rather than re-publishing blindly. If it never appears, the publish \
                 was a silent no-op: check the registry credentials/config and that `{package}` is \
                 a correctly-declared crates.io target. The cut fails here rather than record a \
                 receipt for a publish it cannot confirm"
            ),
            Self::DigestMismatch {
                package,
                version,
                local,
                remote,
            } => write!(
                f,
                "refusing to skip the publish of `{package}@{version}`: it is already on the \
                 registry, but the crate published there (sha256 {remote}) is NOT byte-identical \
                 to the artifact this cut would upload (sha256 {local}). The registry holds a \
                 different artifact at this version than this cut intended — investigate (a \
                 non-reproducible build/toolchain, or a supply-chain substitution) before \
                 proceeding. The cut fails here rather than skip and record a receipt for a crate \
                 it did not publish"
            ),
            Self::DelegatedVersionAlreadyPublished { package, version } => write!(
                f,
                "`{package}@{version}` is already published on the registry, and this target's \
                 publish is CI-delegated (`cargo-publish-ci`). The tag-triggered workflow would \
                 fail with `crate version already uploaded`, while the engine's post-cut verify \
                 would observe the ALREADY-published version and report the cut green — a publish \
                 that never happened. Bump the version (or drop the target) and re-plan; refusing \
                 now, before the tag is pushed"
            ),
            Self::UnsupportedRegistry { adapter, registry } => write!(
                f,
                "adapter `{}` does not support publishing to registry `{}`; it publishes only to \
                 crates.io. Refusing before any publish rather than risk landing on the wrong \
                 registry — fix the target's `registry` in the contract",
                adapter.as_str(),
                registry.as_str()
            ),
        }
    }
}

impl std::error::Error for AdapterError {}

impl AdapterError {
    /// Whether a post-tag distribution failure is safe to retry after bounded
    /// backoff. The command must be the tap clone: it is a local setup/read whose
    /// failure cannot have mutated the remote destination. Later commands (commit,
    /// push, PR creation) have ambiguous remote disposition and are never blindly
    /// repeated, even when their stderr contains a transient-looking status.
    #[must_use]
    pub(crate) fn is_retryable_dist_setup_failure(&self) -> bool {
        let Self::Command {
            command, stderr, ..
        } = self
        else {
            return false;
        };
        if !command.starts_with("gh repo clone ") {
            return false;
        }
        let detail = stderr.to_ascii_lowercase();
        [
            "http 429",
            "http 500",
            "http 502",
            "http 503",
            "http 504",
            "connection reset",
            "connection timed out",
            "operation timed out",
            "temporary failure",
            "temporarily unavailable",
        ]
        .iter()
        .any(|signal| detail.contains(signal))
    }
}

/// The per-ecosystem operations the release coordinator drives through its phase
/// barriers (ADR-0002 §1).
///
/// Deliberately has **no `tag()`** — the shared git tag and GitHub Release are
/// owned by the coordinator alone, which is what makes "tag once, after every
/// publish" a structural guarantee rather than a discipline.
pub trait ReleaseAdapter {
    /// The adapter identity this implementation operates as (a single struct may
    /// back several related identities, e.g. `cargo-publish` and `cargo-dist`).
    fn adapter(&self) -> Adapter;

    /// Whether this adapter's publish is **CI-delegated** — its release artifact
    /// is produced out-of-band by the tag-triggered CI (e.g. `cargo-dist`'s
    /// `release.yml`, a `release-please` merge job, `PyPI`'s trusted-publisher
    /// workflow), never by the engine's [`publish`](Self::publish) step from this
    /// host. The coordinator **skips** such a target in publish-all (journalling a
    /// `target_delegated` fact) rather than calling `publish` and treating its
    /// honest [`AdapterError::Unsupported`] as a phase failure — which would leave
    /// the cut stuck after an irreversible crates.io publish.
    ///
    /// This is a first-class capability the coordinator branches on; it is **not**
    /// inferred from a `publish` that returns [`AdapterError::Unsupported`]. An
    /// adapter that returns `Unsupported` without declaring itself CI-delegated is
    /// a genuine error and still fails the cut. The invariant every CI-delegated
    /// adapter upholds: `is_ci_delegated()` ⇒ `publish` returns
    /// [`AdapterError::Unsupported`].
    ///
    /// Defaults to `false` (the engine owns the publish); the four delegated
    /// identities (`cargo-dist`, `cargo-publish-ci`, `release-please`,
    /// `gh-action-pypi-publish`) override it.
    fn is_ci_delegated(&self) -> bool {
        false
    }

    /// Whether this adapter's tag-triggered CI **owns the shared GitHub Release** —
    /// its workflow creates and finalizes the Release object (and uploads the
    /// cross-platform binaries into it), so the coordinator must NOT create the
    /// Release itself or the two clash over the same tag
    /// (`coordinator-release-vs-cargo-dist-ownership`).
    ///
    /// This is a **strict subset** of [`is_ci_delegated`](Self::is_ci_delegated), not
    /// a synonym: an adapter can be CI-delegated for its *publish* yet not own the
    /// GitHub Release. `gh-action-pypi-publish` uploads to **`PyPI`** (not GitHub),
    /// `cargo-publish-ci` uploads to **crates.io**, and `release-please` is
    /// publish-on-merge — none runs `gh release create` for this tag, so for those
    /// the coordinator still creates the Release. Only
    /// `cargo-dist`, whose generated `release.yml` runs `gh release create <tag> …
    /// artifacts/*` (a create, not an upsert — it errors if the Release pre-exists),
    /// overrides this to `true`. Defaults to `false` (the coordinator owns the
    /// Release, the ADR-0002 default).
    fn ci_owns_github_release(&self) -> bool {
        false
    }

    /// Re-runnable, side-effect-free preview: the exact commands a real cut
    /// would run for `target`.
    ///
    /// # Errors
    /// Returns [`AdapterError`] only if constructing the preview itself fails;
    /// building a preview does not execute the planned commands.
    fn dry_run(
        &self,
        ctx: &EffectCtx<'_>,
        target: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError>;

    /// Re-runnable build of the target's publishable artifacts.
    ///
    /// # Errors
    /// Returns [`AdapterError`] if a build command fails or is unsupported.
    fn build(
        &self,
        ctx: &EffectCtx<'_>,
        target: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError>;

    /// **Per-target irreversible** publish; returns the durable
    /// [`PublishReceipt`].
    ///
    /// # Errors
    /// Returns [`AdapterError`] if a publish command fails or the publish is
    /// unsupported from this host.
    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        target: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError>;

    /// Read-only remote reconcile of a receipt against registry state.
    ///
    /// The default implementation queries [`RegistryQuery`] by the receipt's
    /// ecosystem + package and classifies via [`classify_receipt`]; a lookup
    /// failure yields [`VerifyOutcome::Unknown`]. Homebrew and GitHub Release
    /// adapters override this with their destination-specific read-only observers.
    ///
    /// # Errors
    /// The default never errors (an outage is [`VerifyOutcome::Unknown`], not an
    /// `Err`); the fallible signature lets an override that shells out report a
    /// genuine command failure.
    fn verify(
        &self,
        ctx: &EffectCtx<'_>,
        receipt: &PublishReceipt,
    ) -> Result<VerifyOutcome, AdapterError> {
        Ok(verify_via_registry(ctx, receipt))
    }

    /// Mandatory wall-clock ceiling for a single publish of this adapter — a
    /// hung publish must not wedge a run (ADR-0002 §1).
    fn timeout(&self) -> Duration;
}

/// The enum-backed registry: the six compiled-in ecosystem adapters, selected at
/// runtime from the contract's [`Adapter`] identity by [`resolve`].
///
/// An enum (not an unconstrained `Vec<&dyn ReleaseAdapter>`) so wiring is
/// compiler-checked: [`resolve`]'s match is exhaustive over every [`Adapter`]
/// variant, and a new ecosystem is a new variant the compiler forces you to
/// wire. Implements [`ReleaseAdapter`] by delegating to the resolved inner
/// adapter, giving the coordinator one uniform dispatch type.
pub enum EcosystemAdapter {
    /// The rust ecosystem (`cargo-publish` / `cargo-dist`).
    Rust(cargo::CargoAdapter),
    /// The node ecosystem (`release-please` / `changesets` / `npm-publish`).
    Node(node::NodeAdapter),
    /// The python ecosystem (`gh-action-pypi-publish` / `twine`).
    Python(python::PythonAdapter),
    /// The go ecosystem (`goreleaser`).
    Go(go::GoAdapter),
    /// The homebrew distribution target (`homebrew-tap` / `homebrew-core`).
    Homebrew(homebrew::HomebrewAdapter),
    /// The binary distribution target (`manual` / GitHub Releases).
    Binary(binary::BinaryAdapter),
}

/// Resolve an [`Adapter`] identity to its compiled-in ecosystem implementation.
///
/// The match is **exhaustive** over the adapter enum, so every identity is wired
/// at compile time and a `resolve` for a target can never fail at runtime — the
/// "fail fast at startup, never mid-release" property of ADR-0002 §1.
#[must_use]
pub fn resolve(adapter: Adapter) -> EcosystemAdapter {
    match adapter {
        Adapter::CargoPublish | Adapter::CargoPublishCi | Adapter::CargoDist => {
            EcosystemAdapter::Rust(cargo::CargoAdapter::new(adapter))
        }
        Adapter::ReleasePlease | Adapter::Changesets | Adapter::NpmPublish => {
            EcosystemAdapter::Node(node::NodeAdapter::new(adapter))
        }
        Adapter::GhActionPypiPublish | Adapter::Twine => {
            EcosystemAdapter::Python(python::PythonAdapter::new(adapter))
        }
        Adapter::Goreleaser => EcosystemAdapter::Go(go::GoAdapter::new(adapter)),
        Adapter::HomebrewTap | Adapter::HomebrewCore => {
            EcosystemAdapter::Homebrew(homebrew::HomebrewAdapter::new(adapter))
        }
        Adapter::Manual => EcosystemAdapter::Binary(binary::BinaryAdapter::new(adapter)),
    }
}

impl EcosystemAdapter {
    /// The resolved inner adapter as a trait object, for uniform delegation.
    fn inner(&self) -> &dyn ReleaseAdapter {
        match self {
            Self::Rust(a) => a,
            Self::Node(a) => a,
            Self::Python(a) => a,
            Self::Go(a) => a,
            Self::Homebrew(a) => a,
            Self::Binary(a) => a,
        }
    }
}

impl ReleaseAdapter for EcosystemAdapter {
    fn adapter(&self) -> Adapter {
        self.inner().adapter()
    }
    fn is_ci_delegated(&self) -> bool {
        self.inner().is_ci_delegated()
    }
    fn ci_owns_github_release(&self) -> bool {
        self.inner().ci_owns_github_release()
    }
    fn dry_run(
        &self,
        ctx: &EffectCtx<'_>,
        target: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        self.inner().dry_run(ctx, target)
    }
    fn build(
        &self,
        ctx: &EffectCtx<'_>,
        target: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        self.inner().build(ctx, target)
    }
    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        target: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        self.inner().publish(ctx, target)
    }
    fn verify(
        &self,
        ctx: &EffectCtx<'_>,
        receipt: &PublishReceipt,
    ) -> Result<VerifyOutcome, AdapterError> {
        self.inner().verify(ctx, receipt)
    }
    fn timeout(&self) -> Duration {
        self.inner().timeout()
    }
}

/// What a read-only remote reconcile observed for a receipt's coordinates.
///
/// Constructed by a successful [`RegistryQuery`] lookup; the *absence* of an
/// observation (`None` at the [`classify_receipt`] call site) means the lookup
/// itself failed and classifies as [`VerifyOutcome::Unknown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteObservation {
    /// Versions the registry reports as published for the package.
    pub published_versions: Vec<String>,
    /// The remote digest for the receipt's version, when the registry exposes
    /// one. `None` when the registry cannot be asked for a digest (the current
    /// [`RegistryQuery`] port lists versions only), which makes a digest-level
    /// [`VerifyOutcome::Conflicts`] undetectable — presence still resolves.
    pub remote_digest: Option<String>,
}

/// Classify a [`PublishReceipt`] against an optional remote observation — the
/// pure core of every adapter's `verify` (ADR-0002 §1, ADR-0003 state table).
///
/// - `observed == None` (the lookup could not be performed) ⇒
///   [`VerifyOutcome::Unknown`] — an outage is **never** read as `Missing`.
/// - version absent from the remote set ⇒ [`VerifyOutcome::Missing`].
/// - version present, both digests known and unequal ⇒
///   [`VerifyOutcome::Conflicts`].
/// - version present, digests equal or a digest is unobservable ⇒
///   [`VerifyOutcome::Matches`].
#[must_use]
pub fn classify_receipt(
    receipt: &PublishReceipt,
    observed: Option<&RemoteObservation>,
) -> VerifyOutcome {
    let Some(obs) = observed else {
        return VerifyOutcome::Unknown;
    };
    if !obs.published_versions.iter().any(|v| v == &receipt.version) {
        return VerifyOutcome::Missing;
    }
    match (&receipt.digest, &obs.remote_digest) {
        (Some(local), Some(remote)) if local != remote => VerifyOutcome::Conflicts,
        _ => VerifyOutcome::Matches,
    }
}

/// The default `verify` path: query [`RegistryQuery`] and classify. A lookup
/// error becomes [`VerifyOutcome::Unknown`] (never a false `Missing`).
pub(crate) fn verify_via_registry(ctx: &EffectCtx<'_>, receipt: &PublishReceipt) -> VerifyOutcome {
    let observed = match ctx
        .registry
        .published_versions(receipt.ecosystem.as_str(), &receipt.package)
    {
        Ok(versions) => Some(RemoteObservation {
            published_versions: versions,
            remote_digest: None,
        }),
        Err(_) => None,
    };
    classify_receipt(receipt, observed.as_ref())
}

/// Run a sequence of commands in order through the injected runner, in the
/// repo root, short-circuiting on the first non-zero exit or spawn failure.
pub(crate) fn run_all(
    ctx: &EffectCtx<'_>,
    commands: &[PlannedCommand],
) -> Result<Vec<CommandOutput>, AdapterError> {
    let mut outputs = Vec::with_capacity(commands.len());
    for cmd in commands {
        let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        let out = ctx
            .runner
            .run(&cmd.program, &args, ctx.repo_root)
            .map_err(|e| AdapterError::Io {
                command: cmd.rendered(),
                source: e.to_string(),
            })?;
        if out.status != Some(0) {
            // Many CLIs (npm, go, cargo) write fatal diagnostics to stdout, not
            // stderr — fold stdout in when stderr is empty so the failure is
            // never opaque.
            let detail = if out.stderr.trim().is_empty() {
                out.stdout
            } else {
                out.stderr
            };
            return Err(AdapterError::Command {
                command: cmd.rendered(),
                code: out.status,
                stderr: detail,
            });
        }
        outputs.push(out);
    }
    Ok(outputs)
}

/// Hash the file at `path` (absolute, or relative to the runner's cwd — the repo
/// root) with a SHA-256 CLI, returning the lowercase 64-hex digest.
///
/// Cross-platform: tries `sha256sum` (GNU coreutils — the Linux default) then
/// `shasum -a 256` (Perl — the macOS default), so a cut works on both (`shasum`
/// alone is absent on many Linux hosts). Both print the digest as the first
/// whitespace token, which [`parse_sha256_hex`] extracts; a missing tool (spawn
/// error) or non-zero exit falls through to the next candidate. Shared by the
/// coordinator's source-tarball hash and the cargo adapter's resume-skip
/// digest-authentication.
///
/// `--` terminates option parsing so a `path` beginning with `-` can never be read
/// as a flag (both tools honor it), keeping this shared utility safe for any caller.
pub(crate) fn hash_file(ctx: &EffectCtx<'_>, path: &str) -> Result<String, String> {
    let candidates: [(&str, Vec<&str>); 2] = [
        ("sha256sum", vec!["--", path]),
        ("shasum", vec!["-a", "256", "--", path]),
    ];
    let mut last = String::from("no SHA-256 tool succeeded");
    for (program, args) in &candidates {
        match ctx.runner.run(program, args, ctx.repo_root) {
            Ok(out) if out.status == Some(0) => match parse_sha256_hex(&out.stdout) {
                Some(digest) => return Ok(digest),
                None => {
                    last = format!(
                        "`{program}` produced no parseable sha256: {:?}",
                        out.stdout.trim()
                    );
                }
            },
            Ok(out) => {
                last = format!(
                    "`{program}` exited {}",
                    out.status
                        .map_or_else(|| "signal".to_string(), |c| c.to_string())
                );
            }
            Err(e) => last = format!("cannot run `{program}`: {e}"),
        }
    }
    Err(format!(
        "could not compute the sha256 of `{path}` (tried sha256sum, shasum): {last}"
    ))
}

/// Extract the first whitespace-delimited 64-hex token from a SHA-256 CLI's stdout,
/// lowercased — the digest `sha256sum`/`shasum` both print first (`<hex>  <file>`).
/// `None` when no such token is present (an unexpected output shape).
pub(crate) fn parse_sha256_hex(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .find(|tok| tok.len() == 64 && tok.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

/// Build a [`PublishReceipt`] for `target`, stamping the time from the injected
/// clock — the one place a receipt's fact fields are assembled, shared by every
/// adapter's `publish` so the shape stays uniform.
pub(crate) fn make_receipt(
    ctx: &EffectCtx<'_>,
    target: &AdapterTarget,
    digest: Option<String>,
    remote_url: Option<String>,
) -> PublishReceipt {
    PublishReceipt {
        adapter: target.target.adapter,
        ecosystem: target.ecosystem(),
        package: target.package.clone(),
        version: target.version.clone(),
        canonical_ref: target.canonical_ref(),
        digest,
        remote_url,
        timestamp: ctx.clock.now_unix(),
    }
}

#[cfg(test)]
mod tests;
