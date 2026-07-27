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
/// references keeps adapters testable with recording fakes and unable to touch
/// the real filesystem, network, or clock directly.
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
    /// The adapter has no real implementation of this operation from this host
    /// (e.g. a CI-only trusted-publisher publish). Named so the coordinator can
    /// surface a precise, honest message rather than a fabricated receipt.
    Unsupported {
        /// The adapter identity.
        adapter: Adapter,
        /// The operation that is unsupported (`"publish"`, `"build"`, …).
        operation: &'static str,
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
            Self::Unsupported { adapter, operation } => write!(
                f,
                "adapter `{}` does not support `{operation}` from this host",
                adapter.as_str()
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
