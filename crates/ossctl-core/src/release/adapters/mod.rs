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
//! [`CommandRunner`](crate::ports::CommandRunner),
//! [`Clock`](crate::ports::Clock),
//! [`RegistryQuery`](crate::ports::RegistryQuery)) so an adapter is unit-testable
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

use crate::contract::schema::{Adapter, Ecosystem, Target};
use crate::ports::{Clock, CommandOutput, CommandRunner, RegistryQuery};
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
/// **One scoped exception:** the [`homebrew`] first-formula create writes the
/// generated `.rb` directly with `std::fs` (a new formula *is* a committed file;
/// there is no "add a formula" CLI to route through [`CommandRunner`]). That write
/// is confined to a private, unpredictable scratch checkout it just cloned, uses
/// create-new semantics, and is the sole direct-fs effect in the layer. A general
/// filesystem port on this context is the cleaner long-term home (tracked as issue
/// `homebrew-adapter-fs-port`); until then the exception is deliberate and local.
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
    /// adapter repackages. [`ReleaseArtifacts::EMPTY`] during the re-runnable
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
}

/// The concrete release artifacts the coordinator threads from the build phase
/// into every adapter's [`publish`](ReleaseAdapter::publish) (ADR-0002 §2).
///
/// The two distribution adapters that repackage *already-produced* outputs need
/// inputs no single ecosystem build yields on its own:
/// [`binary`](self::binary) uploads the asset paths gathered from **every**
/// target's [`build`](ReleaseAdapter::build), and [`homebrew`](self::homebrew)'s
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
    /// distribution target ([`binary`](self::binary) or [`homebrew`](self::homebrew)).
    /// The [`binary`](self::binary) adapter records the GitHub-Release page URL for
    /// this slug as its receipt's [`PublishReceipt::remote_url`](crate::protocol::release::PublishReceipt::remote_url).
    /// `None` for a cut with no such target or no resolvable GitHub remote.
    pub repo_slug: Option<String>,
    /// The Homebrew formula inputs the [`homebrew`](self::homebrew) adapter's
    /// first-formula bootstrap needs beyond the [`Self::source_tarball`] URL +
    /// sha256 — the destination tap and the SPDX license the generated `.rb`
    /// records. `None` for a cut with no homebrew target (and always `None` for
    /// every other adapter, which never reads it). Threaded from the plan by the
    /// coordinator alongside [`Self::source_tarball`].
    pub homebrew: Option<HomebrewFormula>,
}

/// The Homebrew-formula inputs a first-formula *create* needs that the source
/// tarball alone does not carry — the destination tap and the formula's license.
///
/// The [`homebrew`](self::homebrew) adapter chooses its **create** vs **bump**
/// path from whether the target formula already exists in [`Self::tap`]; a
/// `None` tap (a `homebrew-core` target, or a `homebrew-tap` the contract left
/// unconfigured) has no bootstrap destination, so the adapter falls back to the
/// plain `bump-formula-pr` path. Like the rest of [`ReleaseArtifacts`] this is an
/// in-memory coordinator↔adapter hand-off, never serialized, so it carries no
/// schema version of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HomebrewFormula {
    /// The destination tap repo as an `owner/repo` slug (from the contract's
    /// `distribution.homebrew_tap`), or `None` when the contract configured none.
    pub tap: Option<String>,
    /// The SPDX license expression the generated formula's `license` stanza
    /// records, or `None` to omit the stanza.
    pub license: Option<String>,
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
};

/// The published source tarball a Homebrew formula bump consumes (`--url` /
/// `--sha256`).
///
/// The `url` is the deterministic GitHub source-archive URL for the cut's tag.
/// The `sha256` is currently always `None` (the formula bump omits `--sha256` and
/// lets `brew` compute it from `--url`): the tag archive the `url` points at is
/// created only in the tag-once phase, *after* publish-all (ADR-0002 §2), so it
/// cannot be fetched-and-hashed here, and a local `git archive` is not byte-equal
/// to GitHub's served tarball — a wrong `--sha256` is worse than none. See
/// [`super::coordinator`]'s `source_tarball` for the full rationale and the
/// post-tag follow-up that would populate a correct digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTarball {
    /// The source tarball's public URL (the GitHub tag archive).
    pub url: String,
    /// The tarball's sha256, once a correct value can be produced (see the type
    /// docs). Currently always `None` — `brew` derives it from [`Self::url`].
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
    /// [`homebrew`](super::homebrew) first-formula create writes the generated
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
    /// workspace dependency is index-visible; see [`cargo`](super::cargo)).
    IndexTimeout {
        /// The published package still absent from the index.
        package: String,
        /// The version being waited for.
        version: String,
        /// How long the wait lasted before giving up, in seconds.
        waited_secs: u64,
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
                "`{package}@{version}` did not appear on the registry index within \
                 {waited_secs}s after publishing; a dependent crate cannot be published \
                 until it is visible"
            ),
        }
    }
}

impl std::error::Error for AdapterError {}

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
    /// failure yields [`VerifyOutcome::Unknown`]. Adapters whose destination is
    /// not observable through [`RegistryQuery`] (homebrew taps, GitHub Releases)
    /// override this to return [`VerifyOutcome::Unknown`] explicitly.
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
        Adapter::CargoPublish | Adapter::CargoDist => {
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
