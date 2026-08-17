//! Rust ecosystem adapter: `cargo-publish` / `cargo-publish-ci` (crates.io) and
//! `cargo-dist`.
//!
//! `cargo-publish` publishes a crate to crates.io via `cargo publish`.
//! `cargo-dist` plans and builds distributable binaries locally (`dist`), but
//! its *upload* is the CI release workflow — so its publish body is
//! [`AdapterError::Unsupported`] from this host rather than a fabricated receipt
//! for a build-only command. `verify` (for `cargo-publish`) reconciles against
//! crates.io through [`RegistryQuery`](crate::ports::RegistryQuery) via the
//! adapter's default path.
//!
//! ## `cargo-publish-ci` — the crates.io publish runs in CI, not here
//!
//! `cargo-publish-ci` is `cargo-publish`'s **CI-delegated** identity, for a repo
//! whose release model is "push the version tag; a tag-triggered workflow runs
//! `cargo publish` with the repo's registry secret" (glasspad's
//! `publish-crates.yml`, the common publish-from-CI-not-a-laptop pattern). Such a
//! repo deliberately forbids the local publish: the maintainer's
//! `~/.cargo/credentials.toml` may be stale (403) or absent, and the CI token is
//! the source of truth — so an engine cut that ran `cargo publish` here would
//! either fail or race the workflow into a double-publish.
//!
//! It differs from `cargo-publish` in **exactly one** respect: the publish. Its
//! `dry_run` and `build` run the identical local gates (`cargo check`, `cargo
//! package --no-verify`), because those are read-only preflights whose whole value
//! is catching an unpublishable manifest *before* the irreversible tag push — and a
//! repo that publishes from CI still wants them. Its `publish` is
//! [`AdapterError::Unsupported`], and it declares
//! [`is_ci_delegated`](ReleaseAdapter::is_ci_delegated), so the coordinator journals
//! `target_delegated` and skips it in publish-all (never publishing, never failing).
//! It does **not** own the GitHub Release (it uploads to crates.io), so a plan
//! carrying only this delegated identity still gets an engine-created Release.
//!
//! The result is a cut whose terminal *actionable* phase is the tag push, followed
//! by the mandatory verify barrier — which polls the crates.io index until CI's
//! publish is observed (see the coordinator's delegated-verify wait). "Delegated"
//! never means "assumed": an unobserved target still fails the cut.
//!
//! ## One plan target = one publish unit (ADR-0004)
//!
//! Each plan target publishes **exactly its own package** — one target ⇒ one
//! `cargo publish -p <package>`. The [coordinator](super::super::coordinator) owns
//! all cross-target ordering: it cuts same-ecosystem targets in dependency order
//! (a dependency's target before its dependents'), so the adapter never re-orders
//! or re-publishes another target's crate. This removes the earlier
//! closure-per-target model, where two authorities (coordinator + adapter) each
//! computed overlapping publish orderings and the crates.io publish→index lag
//! between them could trigger a *duplicate* `cargo publish` of a shared dependency
//! → a partial-publish trap.
//!
//! A workspace whose crates depend on one another still cannot publish in one
//! shot: crates.io rejects a crate whose sibling dependency is not yet indexed
//! (`no matching package named … found`). So before publishing its own package,
//! the adapter discovers that package's publishable intra-workspace dependencies
//! (read-only `cargo metadata`) and **waits for each to be crates.io-index-visible**
//! (polling the injected [`RegistryQuery`](crate::ports::RegistryQuery), bounded by
//! a timeout) — the dependency's own target, cut earlier, already published it, so
//! this only closes the index-lag window. A crate with no publishable workspace
//! dependencies publishes immediately with no wait.
//!
//! ## Deferred packaging for a `=`-pinned dependent (cargo-interleave, ADR-0002)
//!
//! A dependent crate that pins its workspace dependency by exact version
//! (`dep = "=X.Y.Z"`, the shape `/oss-init` emits) **cannot be `cargo package`d
//! before that dependency is published** — not even with `--no-verify`. `cargo
//! package` resolves the `=`-pinned dependency against the crates.io *index* while
//! preparing the upload (a published `.crate` cannot reference a `path` dep), and
//! that version only lands later, in publish-all. `--no-verify` skips the isolated
//! verify *compile*, but not this index resolution. So a strict `build-all` that
//! packaged every crate up front could never package such a dependent
//! (`release-cut-build-phase-dep-ordering`).
//!
//! The fix scopes the ADR-0002 phase barrier narrowly for cargo. `dry_run` /
//! `build` read the workspace graph and probe the registry (see
//! `unpublished_workspace_deps`), then branch on whether the target depends on a
//! workspace crate **not yet on the crates.io index**:
//!
//! - **No unpublished workspace dep** (a leaf, or a dependent whose workspace deps
//!   are already published — a re-cut): fully packaged pre-publish — `cargo check`
//!   (compile safety net) then `cargo package --no-verify` (produces the `.crate`,
//!   validating the manifest). It CAN be packaged: `cargo package` resolves the dep
//!   against the index it is already on.
//! - **Depends on a not-yet-published workspace crate**: its **packaging is deferred**
//!   to `cargo publish` in publish-all, which packages and publishes as one unit
//!   *after* the dependency is published and index-visible (`build` runs only the
//!   index-independent `cargo check`, which resolves the sibling via its `path`).
//!   This is the "build interleaves with publish" exception the coordinator relies
//!   on: the dependent's package step is intrinsic to its dep-ordered publish, not a
//!   premature global-build step. The pre-publish compile safety net (`cargo check`
//!   over every target) still runs as a global build-all barrier, so a compile error
//!   in the default host build fails before **any** irreversible publish.
//!
//! The registry probe is **fail-closed**: a dep the registry cannot confirm as
//! published defers, so a registry outage never risks a build-all `cargo package`
//! that resolves a `=`-pinned dep against an index it cannot reach.
//!
//! The consequence is a target model where **each publishable crate is its own
//! declared target** (which is what `/oss-init` emits). A multi-crate workspace
//! that wants every crate on crates.io declares every crate as a target; a target
//! whose package depends on a workspace crate that is *not* itself a declared
//! target — and whose required version is not already on the index — times out
//! waiting for that crate, the signal that it must be declared. (If that
//! dependency's version happens to already be published, the wait clears and the
//! publish proceeds; the coverage check that would catch an under-declared plan up
//! front is tracked separately, not owned by the adapter.)

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::time::Duration;

use serde::Deserialize;

use crate::contract::schema::{Adapter, Ecosystem, Registry};
use crate::protocol::release::{BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt};

use super::{
    hash_file, make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, ReleaseAdapter,
};

/// Wall-clock ceiling for a single crate's crates.io index-wait, in seconds.
///
/// crates.io's sparse index is usually visible within seconds of a publish, but
/// the publish→index pipeline can lag under load; a generous per-crate ceiling
/// avoids a spurious failure while still bounding a hung wait so it can never
/// wedge a run.
const INDEX_WAIT_TIMEOUT_SECS: u64 = 300;

/// Interval between crates.io index polls while waiting for a just-published
/// version to appear.
const INDEX_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Cargo's registry alias for crates.io. Used two ways: as the value of every
/// `cargo publish/package --registry <alias>` this adapter emits (so the publish
/// destination is pinned and never resolved from ambient registry config —
/// `registry.default`, `.cargo/config.toml`, `CARGO_REGISTRY_DEFAULT`), and as the
/// token a manifest's `publish` allow-list must contain to be crates.io-publishable
/// (a member restricted to a *different* registry is excluded).
const CRATES_IO_ALIAS: &str = "crates-io";

/// The rust release adapter, operating as `cargo-publish`, `cargo-publish-ci`, or
/// `cargo-dist`.
pub struct CargoAdapter {
    adapter: Adapter,
}

impl CargoAdapter {
    /// Construct for a resolved rust adapter identity (`cargo-publish` /
    /// `cargo-publish-ci` / `cargo-dist`).
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        debug_assert!(matches!(
            adapter,
            Adapter::CargoPublish | Adapter::CargoPublishCi | Adapter::CargoDist
        ));
        Self { adapter }
    }

    /// The cargo `--registry` alias to pin this target's `cargo publish`/`package`
    /// invocations to, derived from the target's declared registry.
    ///
    /// crates.io is the only rust registry ossctl supports today, so any other
    /// declared registry is a misconfiguration that must fail **fast, before any
    /// external action** — never a silent publish to an unexpected destination.
    /// Returns [`CRATES_IO_ALIAS`] for [`Registry::CratesIo`], else
    /// [`AdapterError::UnsupportedRegistry`] tagged with **this** adapter's identity
    /// (`self.adapter`, not a hard-coded value, so a `cargo-dist` caller could never
    /// misreport itself). Threading the flag value through here (rather than
    /// hard-coding it at each call site) keeps the destination tied to the contract's
    /// `registry` field and the rejection in one place.
    fn crates_io_registry(&self, t: &AdapterTarget) -> Result<&'static str, AdapterError> {
        match t.target.registry {
            Registry::CratesIo => Ok(CRATES_IO_ALIAS),
            registry => Err(AdapterError::UnsupportedRegistry {
                adapter: self.adapter,
                registry,
            }),
        }
    }
}

impl ReleaseAdapter for CargoAdapter {
    fn adapter(&self) -> Adapter {
        self.adapter
    }

    fn is_ci_delegated(&self) -> bool {
        // `cargo-dist` builds distributables locally but its *upload* is the
        // tag-triggered `release.yml`; `cargo-publish-ci` declares that this repo's
        // crates.io publish is likewise a tag-triggered CI job holding the registry
        // secret. For both, the engine cannot (and must not) publish from this host.
        // `cargo-publish` is a real host publish and is not delegated. Consistent
        // with `publish` returning `Unsupported` for exactly these two identities.
        matches!(self.adapter, Adapter::CargoDist | Adapter::CargoPublishCi)
    }

    fn ci_owns_github_release(&self) -> bool {
        // `cargo-dist`'s generated `release.yml` runs `gh release create <tag> …
        // artifacts/*` — it creates AND finalizes the shared GitHub Release and
        // uploads the cross-platform binaries. So the coordinator must not create
        // the Release itself (a pre-existing Release makes `gh release create`
        // error). Neither `cargo-publish` nor `cargo-publish-ci` owns a GitHub
        // Release — the latter is CI-delegated, but to crates.io, not GitHub, which
        // is exactly why this capability is narrower than `is_ci_delegated`. See
        // `coordinator-release-vs-cargo-dist-ownership`.
        matches!(self.adapter, Adapter::CargoDist)
    }

    fn dry_run(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        if matches!(self.adapter, Adapter::CargoDist) {
            return Ok(DryRunReport {
                adapter: self.adapter,
                planned_commands: vec![PlannedCommand::new(
                    "dist",
                    &["plan", "--output-format=json"],
                )],
                notes: vec![],
            });
        }
        // Reject a non-crates.io target before doing anything — the dry-run
        // preflight must exercise the exact registry-pinned build a real cut runs, and
        // a misconfigured registry is a fail-fast error, not a plannable command.
        let registry = self.crates_io_registry(t)?;
        // FAITHFUL PREFLIGHT: actually run the SAME index-independent build gate the
        // build phase runs, so a plan that cannot compile (or, for a leaf, cannot
        // package) fails HERE at dry-run-all, before any external effect, rather than
        // passing dry-run and failing mid-cut in build-all. (The old dry-run only
        // *described* a `cargo publish --dry-run` without running it, so a build-phase
        // failure slipped past the preflight.)
        //
        // `target_workspace_deps` runs read-only `cargo metadata` first, validating
        // the target is a publishable member and listing its publishable workspace
        // dependencies. `unpublished_workspace_deps` then probes the registry for each
        // (fail-closed) to decide the gate ([`cargo_build_gate`]): a target with **no
        // workspace dep still absent from the index** is packaged now (`cargo package
        // --no-verify`, validating the manifest); a dependent on a **not-yet-published**
        // workspace crate DEFERS packaging to its own `cargo publish` — it cannot be
        // `cargo package`d until that dep is on the index — so the preflight is the
        // index-independent `cargo check` compile gate alone (the check resolves the
        // sibling via its on-disk `path`, never the index), and never false-fails on
        // the unpublished dep. The real end-to-end verify happens in publish-all's
        // `cargo publish`, after the dep is index-visible.
        //
        // The gate COMMANDS are local + self-overwriting (a package writes
        // `target/package/`, the same artifact build-all produces; `check` only warms
        // `target/`), so dry-run stays re-runnable and free of any *external* side
        // effect (ADR-0002). One plan target = one publish unit: exactly this target's
        // own package.
        let deps = target_workspace_deps(ctx, t)?;
        // PRE-TAG BASELINE for a CI-delegated publish. The engine's own publish path
        // probes the registry before uploading (`is_published` → skip / digest-
        // authenticate), which also means a version that is ALREADY on crates.io can
        // never be silently re-published by the engine. A delegated target has no such
        // probe: publish-all skips it, so nothing checks the version until verify —
        // and verify observes *presence*, which a pre-existing upload satisfies. The
        // reachable failure that closes: cutting a version that is already published
        // (a re-cut, or a manifest that was never bumped). CI's `cargo publish` fails
        // with "crate version already uploaded", the engine observes the OLD upload,
        // and the run goes green over a publish that never happened — silently.
        //
        // So establish the baseline here, in dry-run-all: pre-tag, side-effect-free,
        // and before anything irreversible. Fail-closed on an unreachable registry
        // (`is_published`'s own discipline): if we cannot prove the version is absent
        // now, a later "present" observation proves nothing about this cut.
        if self.adapter == Adapter::CargoPublishCi
            && is_published(ctx, t.ecosystem(), &t.package, &t.version)?
        {
            return Err(AdapterError::DelegatedVersionAlreadyPublished {
                package: t.package.clone(),
                version: t.version.clone(),
            });
        }
        let deferred = unpublished_workspace_deps(ctx, t.ecosystem(), &deps);
        let defer_packaging = !deferred.is_empty();
        let (planned_commands, _artifacts) =
            cargo_build_gate(registry, &t.package, &t.version, defer_packaging);
        run_all(ctx, &planned_commands)?;
        // The note states who publishes, which differs by identity: the engine's own
        // publish-all for `cargo-publish`, the tag-triggered workflow for the
        // CI-delegated `cargo-publish-ci` (whose publish-all entry is a journalled
        // skip). An approver reading the dry-run must not be told the engine will run
        // a publish it will never run.
        //
        // Keyed on the IDENTITY, not on `is_ci_delegated()`: that capability is also
        // true for `cargo-dist`, whose CI runs no `cargo publish` and is verified on
        // its GitHub Release. `cargo-dist` returns from its own arm above and never
        // reaches here, so the broad flag reads correctly today — but only by accident
        // of control flow, and this note is what an approver trusts.
        let mut notes = vec![if self.adapter == Adapter::CargoPublishCi {
            format!(
                "publish is CI-delegated: the tag push triggers the workflow that runs \
                 `cargo publish` for `{}`; the engine skips it in publish-all and observes \
                 the crates.io index in verify",
                t.package
            )
        } else {
            format!(
                "publishes with `cargo publish --registry {registry} -p {}` in publish-all",
                t.package
            )
        }];
        if defer_packaging {
            let chain = deferred
                .iter()
                .map(|m| format!("{}@{}", m.name, m.version))
                .collect::<Vec<_>>()
                .join(", ");
            notes.push(format!(
                "packaging of `{}` is deferred to that publish: it depends on workspace \
                 crate(s) not yet on the crates.io index, and a dependent cannot be \
                 `cargo package`d until they are published",
                t.package
            ));
            // The index-wait is the ENGINE's between-publishes wait; a CI-delegated
            // target never runs it (its whole publish happens in the workflow, which
            // owns its own ordering), so promising it would misdescribe the cut.
            notes.push(if self.adapter == Adapter::CargoPublishCi {
                format!(
                    "the CI publish workflow must publish these workspace dependencies of \
                     `{}` first — the engine does not order a delegated publish: {chain}",
                    t.package
                )
            } else {
                format!(
                    "waits for these workspace dependencies to be crates.io-index-visible \
                     before publishing `{}`: {chain}",
                    t.package
                )
            });
        }
        Ok(DryRunReport {
            adapter: self.adapter,
            planned_commands,
            notes,
        })
    }

    fn build(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        // `dist build` emits per-platform tarballs/installers, not a `.crate`;
        // name the artifact set to match what each identity actually produces.
        let (cmds, artifacts, notes) = if matches!(self.adapter, Adapter::CargoDist) {
            (
                vec![PlannedCommand::new("dist", &["build"])],
                vec!["dist/".to_string()],
                vec![],
            )
        } else {
            // Pin `cargo package` to crates.io too (rejecting a non-crates.io target
            // up front) so the build phase can never verify-package against a
            // different registry than the publish phase will target. Reject BEFORE the
            // read-only `cargo metadata` probe so a misconfigured registry runs no
            // command at all.
            let registry = self.crates_io_registry(t)?;
            // Read the workspace graph, then probe the registry to decide the gate: a
            // target that depends on a workspace crate NOT YET on the index cannot be
            // packaged (packaging resolves the `=`-pinned dep against the index), so its
            // packaging is DEFERRED to `cargo publish` and build runs only the
            // index-independent `cargo check`; a target whose workspace deps are already
            // published (or has none) is packaged now. Fail-closed: an unreachable
            // registry defers (see [`unpublished_workspace_deps`]). See
            // [`cargo_build_gate`] and `release-cut-build-phase-dep-ordering`.
            let deps = target_workspace_deps(ctx, t)?;
            let deferred = unpublished_workspace_deps(ctx, t.ecosystem(), &deps);
            let defer_packaging = !deferred.is_empty();
            let (cmds, artifacts) =
                cargo_build_gate(registry, &t.package, &t.version, defer_packaging);
            let notes = if defer_packaging {
                let chain = deferred
                    .iter()
                    .map(|m| format!("{}@{}", m.name, m.version))
                    .collect::<Vec<_>>()
                    .join(", ");
                vec![format!(
                    "packaging of `{}` deferred to `cargo publish` in publish-all (it \
                     depends on workspace crate(s) not yet on the crates.io index: {chain})",
                    t.package
                )]
            } else {
                vec![]
            };
            (cmds, artifacts, notes)
        };
        run_all(ctx, &cmds)?;
        // SKELETON: a production build parses the exact packaged `.crate` /
        // `dist-manifest.json` paths out of the command output; here we name the
        // expected artifact set deterministically.
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts,
            notes,
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // cargo-dist uploads via the CI release workflow, not from this host —
        // `dist build` only builds. `cargo-publish-ci` is the same story for
        // crates.io: the tag-triggered workflow holds the registry token and runs
        // the publish. Report that honestly rather than returning a receipt for a
        // publish that did not happen. Both identities declare `is_ci_delegated`, so
        // the coordinator skips them in publish-all and never reaches this arm; it
        // is the honest answer for any other caller (and the invariant the
        // capability documents: delegated ⇒ `Unsupported`).
        if matches!(self.adapter, Adapter::CargoDist | Adapter::CargoPublishCi) {
            return Err(AdapterError::Unsupported {
                adapter: self.adapter,
                operation: "publish",
            });
        }
        // Reject a non-crates.io target BEFORE the idempotency probe or any publish —
        // the whole publish path (probe, index-wait, receipt URL) assumes crates.io,
        // so a mismatched registry must fail closed here, never reach `cargo publish`.
        let registry = self.crates_io_registry(t)?;
        // PER-TARGET IRREVERSIBLE — drives the real `cargo publish` through the
        // injected runner (the port is the safety seam under test). ADR-0004: one
        // plan target = one publish unit, so this publishes ONLY `t.package`; the
        // coordinator cut every dependency's target before this one. No
        // `--no-verify`: a resume that enters publish without re-running build must
        // still let cargo verify the package before it lands.
        //
        // IDEMPOTENT re-entry with TRI-STATE probing. On resume the coordinator
        // re-enters this method from the top, so probe the registry first and skip
        // an already-landed publish (a second `cargo publish` of an uploaded version
        // hard-fails and would wedge every resume). Crucially, a probe that cannot
        // reach the registry is NOT read as "not published" — that would permit a
        // duplicate upload of a crate that in fact landed. It fails the publish
        // closed ([`AdapterError::RegistryUnavailable`]), mirroring the reconcile
        // layer's outage ⇒ `Unknown` ⇒ never-`Missing` discipline.
        //
        // DIGEST-AUTHENTICATE THE SKIP (`is-published-digest-authenticate`). "Already
        // published" by name + version is NOT proof the crate on the registry is the
        // one this cut intended — a *different* artifact could occupy the version. So
        // before trusting the skip, [`authenticate_skip`] proves the registry's crate
        // is byte-identical to what this cut would upload (its `.crate` sha256 vs the
        // sparse-index `cksum`); only a match skips, a mismatch fails CLOSED, and an
        // outage keeps the same never-guess discipline. This closes the last
        // "receipt without a fresh upload" path (the self-visibility no-op's mirror).
        let ecosystem = t.ecosystem();
        if is_published(ctx, ecosystem, &t.package, &t.version)? {
            return authenticate_skip(ctx, registry, ecosystem, t);
        }
        // crates.io rejects a crate whose sibling dependency is not yet indexed, so
        // wait for this package's own publishable workspace dependencies to be
        // index-visible before publishing it. Each dependency's target was cut
        // earlier by the coordinator; this only closes the publish→index lag window.
        for dep in &target_workspace_deps(ctx, t)? {
            wait_for_index(ctx, ecosystem, &dep.name, &dep.version)?;
        }
        run_all(
            ctx,
            &[PlannedCommand::new(
                "cargo",
                &["publish", "--registry", registry, "-p", &t.package],
            )],
        )?;
        // SELF-VISIBILITY CONFIRM (`cut-noop-self-visibility-check`). `cargo publish`
        // exiting 0 is NOT proof the crate landed: a registry-alias/credential/env
        // difference (or an under-declared target) can make it a silent no-op that
        // ships nothing. Before journaling a receipt, probe the index for this
        // target's OWN `{package, version}` — reusing the bounded index-wait so
        // normal propagation lag is tolerated (only a genuine never-appears no-op
        // fails), and failing closed on a registry outage rather than fabricating a
        // receipt for a publish that may not have happened.
        confirm_self_published(ctx, ecosystem, &t.package, &t.version)?;
        // SKELETON: a production publish parses the crates.io checksum from the
        // `cargo publish` output for `digest`; the canonical URL is well-known. One
        // target publishes exactly one crate, so the journal records exactly one
        // receipt for this crate — `resume`/`verify` track it precisely.
        Ok(make_receipt(ctx, t, None, Some(remote_url(t))))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

/// The build/preflight gate for a `cargo-publish` target — the commands `dry_run`
/// and `build` both run — plus the build artifacts it produces.
///
/// A pure function of the caller's `defer_packaging` decision (computed identically
/// by `dry_run` and `build` from [`unpublished_workspace_deps`], so the two stay in
/// lockstep — a faithful preflight):
///
/// - **`defer_packaging == false`** (a leaf, or a dependent whose workspace deps are
///   already on the index — so it CAN be packaged): `cargo check -p <pkg>` (compile
///   safety net) then `cargo package --registry <r> -p <pkg> --no-verify`. The
///   package validates the manifest and produces the `.crate`, the single build
///   artifact. `--no-verify` skips only the isolated verify *compile* (redundant with
///   the `cargo check` above and re-run for real by `cargo publish` in publish-all).
/// - **`defer_packaging == true`** (a dependent on a workspace crate NOT yet on the
///   index): `cargo check -p <pkg>` **alone** — an index-independent compile (the
///   sibling resolves via its on-disk `path`, never the index). It is the pre-publish
///   safety net that fails a genuine compile error (type/trait/API mismatch, missing
///   item) before any irreversible publish — the partial-publish trap ADR-0004 exists
///   to prevent. Packaging is **deferred** to `cargo publish` in publish-all, which
///   packages+publishes as one unit *after* the dependency is published and
///   index-visible: `cargo package` (even `--no-verify`) resolves the `=X.Y.Z` dep
///   against the crates.io *index* when preparing the upload, so it cannot run until
///   that dependency is published (`release-cut-build-phase-dep-ordering`). No
///   `.crate` is produced here, so the artifact set is empty.
///
/// The gate commands are local + per-target, so no build-time cross-target ordering
/// leaks into the adapter (ADR-0002/0004 preserved); the coordinator alone orders
/// the publishes.
fn cargo_build_gate(
    registry: &str,
    package: &str,
    version: &str,
    defer_packaging: bool,
) -> (Vec<PlannedCommand>, Vec<String>) {
    let mut cmds = vec![PlannedCommand::new("cargo", &["check", "-p", package])];
    if defer_packaging {
        // Deferred packaging: only the index-independent compile gate runs now.
        return (cmds, Vec::new());
    }
    cmds.push(PlannedCommand::new(
        "cargo",
        &[
            "package",
            "--registry",
            registry,
            "-p",
            package,
            "--no-verify",
        ],
    ));
    (cmds, vec![format!("{package}-{version}.crate")])
}

/// The canonical crates.io URL for a target's own package at its version — the
/// receipt's `remote_url`. Correct because the publish paths call this only after
/// [`crates_io_registry`] has confirmed the target's registry is crates.io.
fn remote_url(t: &AdapterTarget) -> String {
    format!("https://crates.io/crates/{}/{}", t.package, t.version)
}

/// The publishable intra-workspace dependencies of the target's own package — the
/// crates that must be crates.io-index-visible before `t.package` can publish
/// (ADR-0004). Each has its own plan target, cut earlier by the coordinator.
///
/// Runs read-only `cargo metadata`, keeps only members publishable to crates.io
/// (dropping `publish = false` and members restricted to another registry), and
/// returns the direct workspace dependencies of `t.package` among them. Errors if
/// `t.package` is not itself a publishable member (the plan approved a package this
/// workspace cannot publish to crates.io). A crate with no publishable workspace
/// dependencies resolves to an empty list — it publishes with no wait.
fn target_workspace_deps(
    ctx: &EffectCtx<'_>,
    t: &AdapterTarget,
) -> Result<Vec<Member>, AdapterError> {
    let meta = load_metadata(ctx)?;
    let members = publishable_members(&meta);
    let by_name: BTreeMap<&str, &Member> = members.iter().map(|m| (m.name.as_str(), m)).collect();
    let Some(target) = by_name.get(t.package.as_str()) else {
        let available: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
        return Err(AdapterError::Command {
            command: "cargo metadata".to_string(),
            code: None,
            stderr: format!(
                "target package `{}` is not a crates.io-publishable member of this workspace \
                 (publishable members: {available:?}); declare each publishable crate as its own \
                 target and check the contract `package` and each crate's `publish` setting",
                t.package
            ),
        });
    };
    // `publishable_members` already restricted each member's `deps` to *other kept
    // members*, so every dep name here resolves to a publishable member.
    Ok(target
        .deps
        .iter()
        .filter_map(|d| by_name.get(d.as_str()).map(|m| (*m).clone()))
        .collect())
}

/// Run `cargo metadata` and parse the workspace graph. Errors on a command
/// failure, on empty output (a real `cargo metadata` never succeeds with empty
/// stdout — empty means a broken host/runner, which must not silently degrade the
/// publish set), or on unparseable output.
fn load_metadata(ctx: &EffectCtx<'_>) -> Result<CargoMetadata, AdapterError> {
    let cmd = PlannedCommand::new("cargo", &["metadata", "--no-deps", "--format-version", "1"]);
    let outputs = run_all(ctx, std::slice::from_ref(&cmd))?;
    let stdout = outputs[0].stdout.trim();
    if stdout.is_empty() {
        return Err(AdapterError::Command {
            command: cmd.rendered(),
            code: None,
            stderr: "`cargo metadata` succeeded but emitted no output — cannot resolve the \
                     workspace publish set"
                .to_string(),
        });
    }
    serde_json::from_str(stdout).map_err(|e| AdapterError::Command {
        command: cmd.rendered(),
        code: None,
        stderr: format!("could not parse `cargo metadata` output: {e}"),
    })
}

/// Project the metadata onto the crates.io-publishable members and their
/// intra-workspace (non-dev) dependency edges.
///
/// A member is kept unless its manifest sets `publish = false` (which
/// `cargo metadata` reports as an empty `publish` array) or restricts publishing
/// to a registry set that does not include crates.io. Only edges to *other kept
/// members* gate order; dev-dependencies are excluded (they never gate publish
/// order and can form legitimate cycles, e.g. a lib crate that dev-depends on the
/// CLI crate for integration tests).
fn publishable_members(meta: &CargoMetadata) -> Vec<Member> {
    let member_ids: HashSet<&str> = meta.workspace_members.iter().map(String::as_str).collect();
    let pkgs: Vec<&MetaPackage> = meta
        .packages
        .iter()
        .filter(|p| member_ids.contains(p.id.as_str()))
        .filter(|p| publishable_to_crates_io(p.publish.as_deref()))
        .collect();
    let names: HashSet<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    pkgs.iter()
        .map(|p| {
            let mut deps: Vec<String> = p
                .dependencies
                .iter()
                // Allow-list the ordering-relevant kinds (normal + build); a future
                // dep kind is excluded rather than accidentally treated as ordering.
                .filter(|d| matches!(d.kind.as_deref(), None | Some("build")))
                .filter(|d| d.name != p.name && names.contains(d.name.as_str()))
                .map(|d| d.name.clone())
                .collect();
            deps.sort();
            deps.dedup();
            Member {
                name: p.name.clone(),
                version: p.version.clone(),
                deps,
            }
        })
        .collect()
}

/// Whether a member's `publish` field permits crates.io. `None`/absent ⇒ any
/// registry (yes); `Some([])` ⇒ `publish = false` (no); `Some([regs…])` ⇒ only if
/// the list names crates.io.
fn publishable_to_crates_io(publish: Option<&[String]>) -> bool {
    match publish {
        None => true,
        Some(regs) => regs.iter().any(|r| r == CRATES_IO_ALIAS),
    }
}

/// The subset of the target's publishable workspace dependencies whose **exact
/// release version is not yet visible on the crates.io index** — the dependencies
/// that make the target unpackageable *now*, so its packaging must defer to
/// `cargo publish` (which packages after those deps publish + index).
///
/// **Fail-closed:** a dependency the registry cannot confirm as published — absent
/// (`Ok(false)`) **or** a registry error (`Err`) — counts as not-yet-published, so
/// packaging defers rather than risk a build-all `cargo package` that resolves a
/// `=`-pinned dep against an index that is missing it or cannot be reached. A
/// dependency **already on the index** (`Ok(true)`) is dropped, so a re-cut whose
/// dependency was published by an earlier release still packages — and manifest-
/// validates — the dependent in build-all (it can: `cargo package` resolves that
/// dep against the index it is already on). This is the precise predicate: "defer
/// iff a workspace dep is not yet on the index", not the coarser "has any workspace
/// dep".
fn unpublished_workspace_deps(
    ctx: &EffectCtx<'_>,
    ecosystem: Ecosystem,
    deps: &[Member],
) -> Vec<Member> {
    deps.iter()
        .filter(|d| !matches!(is_published(ctx, ecosystem, &d.name, &d.version), Ok(true)))
        .cloned()
        .collect()
}

/// Whether `package@version` is already visible on crates.io — the idempotency
/// probe run before `cargo publish` so a resumed cut skips a crate that already
/// landed instead of hard-failing on a duplicate upload.
///
/// **Tri-state, fail-closed.** `Ok(true)` ⇒ already published (skip); `Ok(false)`
/// ⇒ the registry answered and the version is definitively absent (safe to
/// publish); `Err(RegistryUnavailable)` ⇒ the registry could not be reached, so
/// the probe cannot prove the crate has *not* landed. A registry error is **never**
/// read as "not published" (which would permit a duplicate, irreversible upload);
/// the caller fails closed, mirroring the reconcile layer's outage ⇒ `Unknown`
/// discipline.
fn is_published(
    ctx: &EffectCtx<'_>,
    ecosystem: Ecosystem,
    package: &str,
    version: &str,
) -> Result<bool, AdapterError> {
    match ctx.registry.published_versions(ecosystem.as_str(), package) {
        Ok(versions) => Ok(versions.iter().any(|v| v == version)),
        Err(e) => Err(AdapterError::RegistryUnavailable {
            package: package.to_string(),
            version: version.to_string(),
            source: e.to_string(),
        }),
    }
}

/// Authenticate an idempotency skip: `t.package@t.version` is already on the
/// registry, so prove the crate published there is **byte-identical** to what this
/// cut would upload before trusting the skip (`is-published-digest-authenticate`).
///
/// Name + version existence alone is not enough — a *different* artifact could
/// occupy the version (a re-used version from another source, or a supply-chain
/// substitution). So this compares two digests:
///
/// - the **intended** digest: the sha256 of the `.crate` this cut would upload,
///   (re)produced deterministically by [`intended_crate_digest`] (the target's
///   published dependencies are on the index, so it packages cleanly);
/// - the **published** digest: the registry-recorded checksum (crates.io
///   sparse-index `cksum`) from [`RegistryQuery::published_checksum`].
///
/// Only a match trusts the skip and journals a receipt carrying the verified
/// digest (no `cargo publish` runs — the crate is already there). A **mismatch**
/// fails **closed** with [`AdapterError::DigestMismatch`] (the registry holds a
/// different artifact than intended). An **outage** — the checksum cannot be read
/// — fails closed with [`AdapterError::RegistryUnavailable`], never trusting a skip
/// it cannot authenticate (the same never-guess discipline as the idempotency
/// probe and self-visibility confirm).
fn authenticate_skip(
    ctx: &EffectCtx<'_>,
    registry: &str,
    ecosystem: Ecosystem,
    t: &AdapterTarget,
) -> Result<PublishReceipt, AdapterError> {
    let local = intended_crate_digest(ctx, registry, &t.package, &t.version)?;
    let remote = ctx
        .registry
        .published_checksum(ecosystem.as_str(), &t.package, &t.version)
        .map_err(|e| AdapterError::RegistryUnavailable {
            package: t.package.clone(),
            version: t.version.clone(),
            source: e.to_string(),
        })?;
    // Re-validate the registry digest at this domain boundary rather than trusting
    // the [`RegistryQuery`] contract blindly: a faulty/future backend returning a
    // non-hex `Ok(..)` must fail CLOSED as unavailable, never be reported as a
    // `DigestMismatch` (which would misattribute a backend bug to a conflicting
    // artifact). The real crates.io impl already validates; this guards the port.
    if !is_sha256_hex(&remote) {
        return Err(AdapterError::RegistryUnavailable {
            package: t.package.clone(),
            version: t.version.clone(),
            source: format!("registry returned a malformed checksum: {remote:?}"),
        });
    }
    // Both digests are lowercase hex; compare case-insensitively for robustness.
    if !local.eq_ignore_ascii_case(&remote) {
        return Err(AdapterError::DigestMismatch {
            package: t.package.clone(),
            version: t.version.clone(),
            local,
            remote,
        });
    }
    // Authenticated: the registry's crate matches what this cut intended, so the
    // publish is safely skipped and the receipt records the verified digest.
    Ok(make_receipt(ctx, t, Some(local), Some(remote_url(t))))
}

/// The sha256 (lowercase hex) of the `.crate` this cut would upload for
/// `package@version` — the *intended* digest an idempotency skip is authenticated
/// against.
///
/// (Re)produces the `.crate` with `cargo package --no-verify` (local, idempotent,
/// self-overwriting — the same artifact build-all produces) so the digest is of the
/// exact bytes `cargo publish` would upload, independent of whether an earlier build
/// phase left the file on disk. The target is already published, so its `=`-pinned
/// workspace dependencies are on the crates.io index and packaging resolves them.
///
/// The packaged `.crate` lands at `<target_directory>/package/<pkg>-<version>.crate`,
/// where `target_directory` is read from `cargo metadata` (honoring
/// `CARGO_TARGET_DIR` / `[build] target-dir` / workspace config — never a hard-coded
/// `<repo>/target`); [`hash_file`] then hashes it cross-platform.
///
/// **Caveat (tracked for the receipt-provenance cluster).** `cargo package` is
/// deterministic only under a fixed toolchain + source tree + index state; a resume
/// under a *different* cargo version can produce different `.crate` bytes for the same
/// sealed commit, so the digest is a faithful "what this toolchain would upload now",
/// not a durable record of the original upload. The regression-free source of the
/// intended digest is a value journaled at the original publish — see
/// `cargo-publish-receipt-provenance-resume-safety`.
fn intended_crate_digest(
    ctx: &EffectCtx<'_>,
    registry: &str,
    package: &str,
    version: &str,
) -> Result<String, AdapterError> {
    // Resolve the real target directory BEFORE packaging so the hash reads the file
    // cargo actually writes, not a hard-coded path a custom target-dir would miss.
    let target_dir = load_metadata(ctx)?.target_directory;
    run_all(
        ctx,
        &[PlannedCommand::new(
            "cargo",
            &[
                "package",
                "--registry",
                registry,
                "-p",
                package,
                "--no-verify",
            ],
        )],
    )?;
    let crate_path = format!("{target_dir}/package/{package}-{version}.crate");
    hash_file(ctx, &crate_path).map_err(|source| AdapterError::Command {
        command: format!("sha256 of {crate_path}"),
        code: None,
        stderr: source,
    })
}

/// Whether `s` is a well-formed lowercase-or-mixed-case 64-char hex SHA-256 — the
/// shape both a crates.io `cksum` and a local `.crate` hash must have. Used to
/// validate a registry-supplied digest at the [`authenticate_skip`] boundary.
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Why a bounded index-wait gave up without ever observing `package@version` — the
/// honest distinction the caller maps onto its own [`AdapterError`] variant.
enum WaitFailure {
    /// The registry *answered at least once* over the window and the version was
    /// definitively absent every time it did — a genuine "did not appear" (the
    /// dependency never indexed, or the crate's own publish shipped nothing).
    Absent {
        /// How long the wait actually lasted before giving up, in seconds.
        waited_secs: u64,
    },
    /// The registry was **never** reached with a definitive answer over the whole
    /// window — every poll errored — so absence could not be established: an outage,
    /// not a proven absence. Carries the last underlying error so it is surfaced,
    /// never masked.
    Unreachable {
        /// The underlying registry lookup error, rendered as text.
        source: String,
    },
}

/// Poll the crates.io index (through the injected [`RegistryQuery`]) until
/// `package@version` is visible, or the per-crate timeout elapses.
///
/// Between polls it waits [`INDEX_POLL_INTERVAL`] through the injected
/// [`Clock::sleep`](crate::ports::Clock::sleep) — real time in production, a
/// virtual advance under test — so the loop is bounded, never busy, and
/// deterministic in tests. A transient lookup error is retried (waiting is
/// reversible), and the outcome on timeout is classified over the **whole window,
/// not just the final poll**: if *any* poll reached the registry and observed the
/// version absent, that is [`WaitFailure::Absent`]; if **no** poll ever got a
/// definitive answer (every one errored), that is [`WaitFailure::Unreachable`]
/// carrying the last error — a sustained outage is never masked as "did not index"
/// just because the last poll happened (or failed) to answer. This fails closed:
/// an all-outage window is `Unreachable`, never a false absence. The two callers
/// ([`wait_for_index`] for a dependency, [`confirm_self_published`] for the crate
/// just published) map these to their own [`AdapterError`] variants, so the same
/// bounded, propagation-lag-tolerant wait backs both.
fn poll_for_index(
    ctx: &EffectCtx<'_>,
    ecosystem: Ecosystem,
    package: &str,
    version: &str,
) -> Result<(), WaitFailure> {
    let start = ctx.clock.now_unix();
    // Whether ANY poll reached the registry and got a definitive answer (a version
    // list, in which the version was absent). Drives the fail-closed classification:
    // a window that never once saw the registry answer is an outage
    // (`Unreachable`), never a proven absence — even if the final poll erred or
    // answered. Only a window that DID observe a clean absence classifies as
    // `Absent`.
    let mut observed_absent = false;
    // The most recent registry error, surfaced when the window was a pure outage.
    let mut last_err: Option<String> = None;
    loop {
        match ctx.registry.published_versions(ecosystem.as_str(), package) {
            Ok(versions) => {
                if versions.iter().any(|v| v == version) {
                    return Ok(());
                }
                // A definitive answer: the registry was reached and the version is
                // absent. `last_err` is intentionally NOT cleared — it is only read on
                // the `Unreachable` path, which is taken solely when `observed_absent`
                // is false (no clean answer ever occurred), so a stale error string
                // can never leak into an `Absent` classification.
                observed_absent = true;
            }
            Err(e) => last_err = Some(e.to_string()),
        }
        let waited = ctx.clock.now_unix().saturating_sub(start);
        if waited >= INDEX_WAIT_TIMEOUT_SECS {
            return Err(if observed_absent {
                WaitFailure::Absent {
                    waited_secs: waited,
                }
            } else {
                WaitFailure::Unreachable {
                    source: last_err.unwrap_or_else(|| {
                        "the registry never returned a definitive answer".to_string()
                    }),
                }
            });
        }
        ctx.clock.sleep(INDEX_POLL_INTERVAL);
    }
}

/// Wait for a **workspace dependency** to be crates.io-index-visible before the
/// dependent's `cargo publish` (crates.io rejects a crate whose sibling dependency
/// is not yet indexed). An absence-after-wait is [`AdapterError::IndexTimeout`] (the
/// dependency never indexed — likely under-declared); an outage is
/// [`AdapterError::RegistryUnavailable`] (fail-closed, never masked as "did not
/// index").
fn wait_for_index(
    ctx: &EffectCtx<'_>,
    ecosystem: Ecosystem,
    package: &str,
    version: &str,
) -> Result<(), AdapterError> {
    poll_for_index(ctx, ecosystem, package, version).map_err(|f| match f {
        WaitFailure::Absent { waited_secs } => AdapterError::IndexTimeout {
            package: package.to_string(),
            version: version.to_string(),
            waited_secs,
        },
        WaitFailure::Unreachable { source } => AdapterError::RegistryUnavailable {
            package: package.to_string(),
            version: version.to_string(),
            source,
        },
    })
}

/// Confirm the crate the adapter **just published** is visible on the index before a
/// receipt is journaled — the self-visibility check that turns an unconfirmed upload
/// into a fail-closed refusal rather than a fabricated success
/// (`cut-noop-self-visibility-check`).
///
/// A `cargo publish` that exits 0 but shipped nothing (a registry-alias/credential/
/// env difference, an under-declared target) would otherwise fabricate a
/// [`PublishReceipt`](crate::protocol::release::PublishReceipt) and report the cut a
/// success while nothing reached crates.io. So after the irreversible upload the
/// publish path probes the registry for the target's *own* `{package, version}`,
/// reusing the same bounded [`poll_for_index`] wait as the dependency index-wait so
/// normal sparse-index propagation lag is tolerated — only a version that never
/// appears within the window fails. That failure is
/// [`AdapterError::PublishNotVisible`] (naming the crate + version): the cut fails
/// **closed** rather than record a receipt it cannot substantiate — the upload may
/// have landed on a slow index (resume/verify) or shipped nothing (a genuine no-op).
/// A registry outage (never reachable across the window) is
/// [`AdapterError::RegistryUnavailable`] instead (fail-closed too, mirroring the
/// reconcile layer's outage discipline).
fn confirm_self_published(
    ctx: &EffectCtx<'_>,
    ecosystem: Ecosystem,
    package: &str,
    version: &str,
) -> Result<(), AdapterError> {
    poll_for_index(ctx, ecosystem, package, version).map_err(|f| match f {
        WaitFailure::Absent { waited_secs } => AdapterError::PublishNotVisible {
            package: package.to_string(),
            version: version.to_string(),
            waited_secs,
        },
        WaitFailure::Unreachable { source } => AdapterError::RegistryUnavailable {
            package: package.to_string(),
            version: version.to_string(),
            source,
        },
    })
}

/// A publishable workspace member with its version and its intra-workspace
/// (non-dev) dependency names.
#[derive(Clone)]
struct Member {
    name: String,
    version: String,
    deps: Vec<String>,
}

/// The subset of `cargo metadata --format-version 1 --no-deps` output the
/// publish-order discovery reads.
#[derive(Deserialize)]
struct CargoMetadata {
    /// Every package in the metadata; with `--no-deps` these are the workspace
    /// members only.
    packages: Vec<MetaPackage>,
    /// The package ids that are workspace members (matched against
    /// [`MetaPackage::id`] to be exact regardless of the id string format).
    workspace_members: Vec<String>,
    /// The absolute build target directory cargo resolved for this invocation —
    /// honoring `CARGO_TARGET_DIR`, `[build] target-dir`, and workspace config, so a
    /// consumer never hard-codes `<repo>/target`. `cargo metadata` always emits it;
    /// defaulted only so the many callers that read only [`Self::packages`] /
    /// [`Self::workspace_members`] can deserialize fixtures without it. The
    /// packaged `.crate` lands under `<target_directory>/package/`.
    #[serde(default)]
    target_directory: String,
}

/// One package entry from `cargo metadata`.
#[derive(Deserialize)]
struct MetaPackage {
    name: String,
    version: String,
    id: String,
    #[serde(default)]
    dependencies: Vec<MetaDep>,
    /// `null`/absent ⇒ publishable to any registry; `[]` ⇒ `publish = false`;
    /// `["<registry>",…]` ⇒ publishable to a restricted set (still publishable).
    #[serde(default)]
    publish: Option<Vec<String>>,
}

/// One dependency entry from `cargo metadata`.
#[derive(Deserialize)]
struct MetaDep {
    name: String,
    /// `null` (normal), `"dev"`, or `"build"`. Only normal/build deps gate
    /// publish order; dev-deps never do.
    #[serde(default)]
    kind: Option<String>,
}
