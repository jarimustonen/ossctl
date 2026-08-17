//! Phase-barrier coordinator: ordering, phase barriers, and tag ownership
//! (ADR-0002 §2).
//!
//! Drives every configured ecosystem adapter through the five barriers
//! **dry-run-all → build-all → publish-all → tag-once → dist (post-tag
//! finalize)**, with tagging owned by the coordinator alone (never an adapter).
//! This is the one stateful,
//! partially-irreversible operation in `ossctl`; the guarantees it enforces are:
//!
//! - **Publish from a clean checkout of the sealed commit
//!   (`release-cut-clean-checkout`).** Before any effect phase, [`execute`]
//!   materializes a throwaway detached `git worktree` at
//!   [`plan.head_sha`](crate::protocol::plan::ReleasePlan::head_sha) and re-roots the
//!   effect context there ([`EffectCtx::with_repo_root`]), so every dry-run / build /
//!   publish / dist command runs against the **approved bytes**, never the operator's
//!   live, mutable working tree. A cut is therefore reproducible and immune to a
//!   mid-cut edit of the tree; it **fails closed** ([`CutError::Checkout`]) if the
//!   sealed commit is not present locally, and the checkout is torn down on every
//!   exit path. The journal (git-common-dir, ADR-0003) and the coordinator-owned tag
//!   stay on the **real** repo — only the adapter commands move.
//! - **Strict barriers.** Every target must clear a phase before *any* target
//!   enters the next. A publish can never precede an all-targets build; a tag can
//!   never precede an all-targets publish. A failure in phase *K* blocks entry to
//!   *K+1* and records a `phase_completed { phase, outcome: failed }` fact.
//! - **One scoped exception — cargo-ecosystem interleave (ADR-0002 amendment,
//!   2026-08-06).** For a multi-crate cargo workspace whose dependent crate pins a
//!   workspace dependency that is **not yet on the crates.io index** (`dep =
//!   "=X.Y.Z"`, the shape `/oss-init` emits, cut in lockstep), the dependent **cannot
//!   be packaged in build-all** — `cargo package` resolves the `=`-pinned dependency
//!   against the index while preparing the upload, and that version is only published
//!   later, in publish-all (`release-cut-build-phase-dep-ordering`). So the cargo
//!   adapter **defers the dependent's packaging into its `cargo publish`**, which
//!   packages+publishes as one unit in the dep-ordered publish phase, *after* the
//!   dependency is published and index-visible. (A dependent whose workspace deps are
//!   already on the index — a re-cut — still packages in build-all; the adapter probes
//!   the registry to decide.) The coordinator does not special-case this: publish-all
//!   already walks same-ecosystem targets in dependency order and the adapter's
//!   `publish` already index-waits on the target's own deps, so `publish core → wait
//!   index → package+publish cli` falls out of the existing dep-ordered publish phase.
//!   The **outer barrier still holds**: dry-run-all runs first (every target, a
//!   `cargo check` for cargo), the pre-publish compile safety net is a global
//!   build-all barrier before **any** publish, tagging is still coordinator-only and
//!   once-after-all-publishes, and the post-tag homebrew phase is unchanged. Only the
//!   dependent's *packaging* interleaves with publish.
//! - **Coordinator-only tagging.** The shared git tag is created and pushed here,
//!   exactly once, only after every publish has succeeded, through the injected
//!   [`Tagger`] port. The three tag steps
//!   (`tag_created_local` → `tag_pushed_remote` → `github_release_created` /
//!   `github_release_delegated`) are independently journalled so an interrupted tag
//!   phase resumes step-by-step. The **GitHub Release** step is conditional on
//!   ownership: for a plan with a target whose CI owns the Release
//!   ([`ci_owns_github_release`](super::adapters::ReleaseAdapter::ci_owns_github_release)
//!   — `cargo-dist`) the tag-triggered CI owns Release creation + the cross-platform
//!   binary upload, so the coordinator pushes the tag (which triggers CI) but journals
//!   `github_release_delegated` and does **not** create the Release — avoiding a
//!   double-create clash. Otherwise the coordinator creates the Release itself
//!   (`github_release_created`), the ADR-0002 default. This is a strict subset of
//!   CI-delegation: a PyPI-trusted-publisher or `release-please` target is
//!   CI-delegated for its *publish* yet does not own the GitHub Release, so those
//!   plans still get an engine-created Release
//!   (`coordinator-release-vs-cargo-dist-ownership`).
//! - **CI-delegated targets are skipped, not failed.** A target whose adapter
//!   declares [`is_ci_delegated`](ReleaseAdapter::is_ci_delegated) (its artifact is
//!   produced by the tag-triggered CI, e.g. `cargo-dist`'s `release.yml`) is
//!   journalled `target_delegated` in publish-all and skipped — never published
//!   from this host, never counted as a failure. This closes the partial-publish
//!   trap where an honest [`AdapterError::Unsupported`](super::adapters::AdapterError::Unsupported)
//!   from such an adapter, after
//!   an irreversible crates.io publish, would wedge the run.
//! - **Post-tag distribution finalize.** Targets whose artifact only *exists*
//!   after the tag is pushed — the Homebrew formula, whose `url` is the just-created
//!   tag archive — are finalized in a fifth **dist** barrier that runs after
//!   tag-once: the coordinator resolves the pushed tag archive, computes its real
//!   `sha256`, and hands it to the Homebrew adapter so the generated `.rb` carries a
//!   correct hash (no draft-PR placeholder). It runs for every cut (a no-op when
//!   there is no post-tag target) and its `Ok` completion flips the run to
//!   [`RunStatus::Completed`](crate::protocol::journal::RunStatus::Completed).
//! - **No auto-rollback.** On any failure the coordinator *stops and journals
//!   precisely what landed* — it never undoes a published artifact. Recovery is
//!   the human's, through `release verify` / `release resume` (wave-3), which read
//!   the durable state this coordinator leaves behind.
//!
//! # Event shape (what resume + `release show` build on)
//!
//! Every state transition is a fact appended to the [`Journal`] via
//! append-then-apply (ADR-0003 §2) and mirrored to the injected [`ProgressSink`]
//! for `--output=jsonl` streaming (§12). The event stream for a clean two-target
//! cut is:
//!
//! ```text
//! run_created
//! phase_entered dry_run ; target_dry_run … ; phase_completed dry_run ok
//! phase_entered build   ; target_built …   ; phase_completed build ok
//! phase_entered publish ; target_published …(receipt each) / target_delegated …(CI-owned) ; phase_completed publish ok
//! phase_entered tag     ; tag_created_local ; tag_pushed_remote ; github_release_created (or github_release_delegated when a CI-delegated target owns the Release) ; phase_completed tag ok
//! phase_entered dist    ; target_published …(homebrew, real sha256) ; phase_completed dist ok
//! ```
//!
//! `run_created` is written by [`Journal::create`] before [`execute`] runs; the
//! final `phase_completed dist ok` is what flips the run to
//! [`RunStatus::Completed`](crate::protocol::journal::RunStatus::Completed) in the
//! reducer.
//!
//! # Resume-readiness (idempotent re-entry)
//!
//! [`execute`] is safe to call on a journal that already carries partial progress
//! (the shape wave-3 `release resume` relies on): a phase already recorded
//! [`PhaseOutcome::Ok`] is skipped whole, and within a re-entered phase a target
//! already in the corresponding projection set (`dry_run` / `built` / `published`)
//! is skipped rather than re-executed. So a cut that failed publishing target *B*
//! after publishing *A* re-runs to complete *B* and tag — **without**
//! re-publishing *A*. (Ground-truth remote reconciliation before a re-publish is
//! wave-3's `reconcile`; this layer provides the journal-driven skip it builds
//! on.)

use crate::ports::{CommandRunner, Tagger};
use crate::protocol::journal::{
    EventKind, JournalEvent, Phase, PhaseOutcome, PublishReceipt as JournalReceipt, RunState,
    JOURNAL_SCHEMA_VERSION,
};
use crate::protocol::plan::ReleasePlan;
use crate::protocol::release::{PublishReceipt as AdapterReceipt, VerifyOutcome};

use super::adapters::{
    hash_file, observe_github_release_assets, resolve, verification_artifacts, AdapterTarget,
    EcosystemAdapter, EffectCtx, HomebrewAsset, HomebrewFormula, ReleaseAdapter, ReleaseArtifacts,
    SourceTarball,
};
use super::journal::Journal;
use super::journal_target_ids;
use crate::contract::schema::{Adapter, Registry, Target};

/// A destination for the coordinator's progress events, so a real cut can stream
/// them (`--output=jsonl`, §12) while the same events are durably journalled.
///
/// The coordinator calls [`Self::event`] with each fact **after** it has been
/// appended to the journal (never before — a streamed event the journal did not
/// commit would be a lie). Use [`NullSink`] when no streaming is wanted (the
/// journal is still the durable record).
pub trait ProgressSink {
    /// Handle one just-journalled event (e.g. write it as a JSONL line).
    fn event(&mut self, event: &JournalEvent);
}

/// A [`ProgressSink`] that discards every event — for callers (and tests) that
/// only care about the durable journal.
pub struct NullSink;

impl ProgressSink for NullSink {
    fn event(&mut self, _event: &JournalEvent) {}
}

/// Why a `release cut` could not complete. Carries enough to render the §10 error
/// envelope **and** to point the operator at recovery: the run's journal already
/// records exactly what landed (there is no rollback), so `release verify
/// <run_id>` / `release resume <run_id>` pick up from here.
#[derive(Debug)]
pub enum CutError {
    /// A phase barrier failed. `target` names the offending target (a per-target
    /// dry-run/build/publish failure) or is `None` for a coordinator-owned tag
    /// step. The run is stopped, the failure is journalled, and nothing is undone.
    PhaseFailed {
        /// The phase whose barrier failed.
        phase: Phase,
        /// The target that failed, or `None` for a coordinator step (tagging).
        target: Option<String>,
        /// The underlying failure, rendered for the operator.
        message: String,
    },
    /// A journal append failed — the run's durable record could not be written,
    /// so the coordinator refuses to proceed (acting without recording is the one
    /// thing worse than stopping).
    Journal(std::io::Error),
    /// The sealed plan could not be turned into executable targets — an
    /// unresolved package name, or two targets that collide on one ecosystem id.
    /// Caught before any external action.
    Plan(String),
    /// The sealed commit ([`ReleasePlan::head_sha`]) could not be materialized as a
    /// clean checkout to publish from — it is not present locally (never committed,
    /// not fetched, or garbage-collected) or the throwaway worktree could not be
    /// created. **Fail-closed**, before any effect phase: a cut publishes from a
    /// fresh checkout of the sealed HEAD (not the live working tree), so if that
    /// commit is unavailable there is nothing safe to publish from
    /// (`release-cut-clean-checkout`). Nothing external has happened.
    Checkout(String),
}

impl std::fmt::Display for CutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PhaseFailed {
                phase,
                target,
                message,
            } => match target {
                Some(t) => write!(
                    f,
                    "{}-phase failed on target `{t}`: {message}",
                    phase.as_str()
                ),
                None => write!(f, "{}-phase failed: {message}", phase.as_str()),
            },
            Self::Journal(e) => write!(f, "could not write the release journal: {e}"),
            Self::Plan(m) => write!(f, "the sealed plan is not executable: {m}"),
            Self::Checkout(m) => {
                write!(
                    f,
                    "could not check out the sealed commit to publish from: {m}"
                )
            }
        }
    }
}

impl std::error::Error for CutError {}

/// One resolved unit of work: a target's journal id, its compiled-in adapter, and
/// the per-target input the adapter operates on.
struct TargetPlan {
    /// The stable journal key for this target (its ecosystem wire string).
    id: String,
    /// The compiled-in adapter resolved from the target's adapter identity.
    adapter: EcosystemAdapter,
    /// The per-target release input (contract slice + resolved package + version).
    input: AdapterTarget,
}

/// Execute a sealed, already-drift-checked `plan` across the four phase barriers,
/// journalling every transition through `journal` and mirroring each fact to
/// `sink`.
///
/// The caller (`release cut`) is responsible for having **refused on drift** (the
/// plan module's `plan_id` re-hash) and for having created `journal` with the
/// matching `RunCreated` event; this function does not re-check the seal — it
/// executes the plan it is handed. `ctx` supplies the injected effect ports each
/// adapter shells out through; `tagger` owns the shared tag.
///
/// # Reproducible cut: publish from a clean checkout of the sealed commit
///
/// Before any effect phase runs, this materializes a fresh, detached checkout of
/// [`plan.head_sha`](ReleasePlan::head_sha) (a temporary `git worktree`) and
/// re-roots `ctx` at it via [`EffectCtx::with_repo_root`], so **every** `dry_run` /
/// `build` / `publish` / dist command runs against the approved bytes — never the
/// operator's live, mutable working tree. This makes a cut reproducible and immune
/// to a mid-cut edit of the tree (the version / self-visibility guards become a
/// property of the sealed commit, not a point-in-time snapshot). The checkout is
/// torn down on **every** exit path (success, phase failure, or panic) by the
/// `SealedCheckout` guard's `Drop`. If the sealed commit is not present locally,
/// the cut **fails closed** with [`CutError::Checkout`] before touching anything.
///
/// The **journal** and the **tag** deliberately do *not* move: the journal stays
/// rooted under the real repo's git-common-dir (ADR-0003; `journal` already carries
/// that path) and `tagger` operates on the real repo (a linked worktree shares the
/// object store, so the tag it creates against `plan.head_sha` is visible
/// everywhere). Only the adapter effect commands are re-rooted.
///
/// ## Caveats of the fresh checkout
///
/// - **Cold builds.** A fresh worktree has no `target/` (or `node_modules/`, …), so a
///   cargo cut recompiles from scratch every time — reproducibility bought at a
///   per-cut build-time cost. A shared `CARGO_TARGET_DIR` cache keyed by the sealed
///   commit is a possible future optimization (tracked as a follow-up).
/// - **Tracked-only bytes.** The checkout is the commit's *tracked* tree: untracked
///   / git-ignored files the operator built against are absent. This is the intended
///   reproducibility property (the published bytes are exactly the sealed commit),
///   but a workspace that git-ignores its `Cargo.lock` publishes without it, and a
///   build-input a CI step generates-but-never-commits will be missing — such inputs
///   must be committed to be part of a cut.
/// - **The common case is fine.** Cutting when `HEAD == plan.head_sha` (the operator
///   committed the release commit, then ran `release cut`) works: git permits a
///   detached worktree at an already-checked-out commit (only a *branch* checked out
///   twice is refused).
///
/// # Errors
/// Returns [`CutError`] on a missing/uncheckout-able sealed commit
/// ([`CutError::Checkout`], fail-closed before any phase), the first phase failure
/// (barrier blocked), a journal write failure, or an unexecutable plan. On a
/// [`CutError::PhaseFailed`] the partial state is already durably journalled —
/// **nothing is rolled back**.
pub fn execute(
    journal: &mut Journal<'_>,
    plan: &ReleasePlan,
    ctx: &EffectCtx<'_>,
    tagger: &dyn Tagger,
    sink: &mut dyn ProgressSink,
) -> Result<(), CutError> {
    // `execute` is public and used by resume as well as fresh cuts, so repeat the
    // no-effects plan validation even when a caller did not run the CLI preflight.
    validate_plan(plan)?;
    let targets = resolve_target_plans(plan)?;

    // Resolve the GitHub `origin` slug from the REAL repo root, BEFORE re-rooting to
    // the checkout. `git remote get-url origin` reads git *config*, not checkout
    // *contents*, so it must run against the real repo: reading it from the throwaway
    // worktree cwd (under `$TMPDIR`) could silently miss the slug under a
    // conditional-include (`includeIf gitdir:`) config or a strict `safe.directory`
    // guard — the exact silent-downgrade (homebrew/binary lose their tarball/URL)
    // this feature exists to prevent (llm-review). The slug depends only on the plan
    // + `origin`, never on build output, so resolving it once up front is correct.
    let repo_slug = resolve_repo_slug(ctx, &targets);

    // The commit every effect phase builds/publishes from. For a fresh cut this is the
    // sealed pre-bump `head_sha` (the bump phase below commits ON TOP of it and moves the
    // checkout to the bump commit). For a RESUME of a --bump run whose bump already landed
    // (`state.bump` recorded), it is the recorded bump commit — so the resumed
    // dry-run/build/publish operate on the BUMPED tree, never the pre-bump one (which
    // would publish the OLD version while the tag points at the bump commit; llm-review
    // consensus critical fix). A no-bump run always uses `head_sha`.
    let checkout_commit = journal
        .state()
        .bump
        .as_ref()
        .map_or_else(|| plan.head_sha.clone(), |b| b.commit.clone());

    // Publish from a CLEAN CHECKOUT of that commit, not the live tree
    // (`release-cut-clean-checkout`). Materialize a throwaway detached `git worktree`
    // (fail-closed if the commit is absent locally), then re-root the effect context there
    // so every adapter command below runs against the approved bytes. The guard tears the
    // worktree down on every exit path (Drop). The journal + tagger stay on the real repo.
    let checkout = SealedCheckout::materialize(ctx, &checkout_commit)?;
    let checkout_ctx = ctx.with_repo_root(checkout.path());
    let ctx = &checkout_ctx;

    // Engine-owned version bump (--bump plans only) — FIRST, before any build/publish,
    // so every later phase builds and publishes the BUMPED tree. On a fresh run it applies
    // the sealed edits in the checkout, runs any bump_hook, commits, and journals the bump
    // commit; on resume (bump already recorded) it is a no-op (the checkout was already
    // materialized AT the bump commit above, so no re-apply and never a double-bump). A
    // no-bump plan is a clean no-op here.
    bump_phase(journal, sink, ctx, plan)?;
    // The commit the tag must point at: the bump commit for a --bump run, else head_sha.
    let tag_commit = journal
        .state()
        .bump
        .as_ref()
        .map_or_else(|| plan.head_sha.clone(), |b| b.commit.clone());

    // The source-tarball URL + homebrew formula inputs depend only on the plan + the
    // already-resolved slug, never on any build output, so the dry-run/build phases
    // preview the *real*, fully parameterized commands (the homebrew adapter needs
    // the tap to even decide create-vs-bump). Only `assets` (the binary upload set)
    // is build-produced, so it is empty for these pre-build phases and accumulated
    // during build-all.
    let source_tarball = repo_slug
        .as_deref()
        .and_then(|slug| source_tarball(slug, plan, &targets));
    let homebrew = homebrew_inputs(plan, &targets);
    let pre_artifacts = ReleaseArtifacts {
        assets: Vec::new(),
        source_tarball: source_tarball.clone(),
        repo_slug: repo_slug.clone(),
        homebrew: homebrew.clone(),
        homebrew_assets: Vec::new(),
    };
    let pre_ctx = ctx.with_artifacts(&pre_artifacts);

    // dry-run-all → build-all: re-runnable, side-effect-free barriers. build-all
    // is where the concrete asset paths become known, so it accumulates them.
    reversible_phase(journal, sink, &pre_ctx, Phase::DryRun, &targets, None)?;
    let mut assets = Vec::new();
    reversible_phase(
        journal,
        sink,
        &pre_ctx,
        Phase::Build,
        &targets,
        Some(&mut assets),
    )?;

    // Thread the build's concrete artifacts into publish-all: the aggregated
    // asset paths (binary) join the already-resolved slug / source-tarball /
    // homebrew inputs.
    //
    // Resume caveat: a resume that skipped a completed build phase re-gathers
    // nothing here (`assets` stays empty), so the binary adapter would see an
    // empty/partial upload set. On a fresh cut the set is complete; making the
    // aggregated build manifest survive resume (journaling it per target) is a
    // documented follow-up. See `threads_no_assets_when_build_phase_is_resumed`
    // for the pinned current behavior.
    let artifacts = ReleaseArtifacts {
        assets,
        source_tarball,
        repo_slug,
        homebrew,
        homebrew_assets: Vec::new(),
    };
    // publish-all: per-target irreversible; receipts journalled per target. The
    // publish phase is the only one that sees the build-complete artifacts. It
    // publishes the engine-owned targets, journals CI-delegated targets as skipped,
    // and defers post-tag targets (homebrew) to the dist phase below.
    publish_phase(journal, sink, &ctx.with_artifacts(&artifacts), &targets)?;
    // tag-once: coordinator-only, only after every publish succeeded. When the plan
    // carries a target whose tag-triggered CI OWNS the GitHub Release (cargo-dist),
    // the coordinator creates + pushes the tag but does NOT create the Release itself
    // (it would clash with CI over the same Release). This is the narrow
    // `ci_owns_github_release()` capability, NOT the broader `is_ci_delegated()`: a
    // PyPI-trusted-publisher or release-please target is CI-delegated for its publish
    // yet does not own the GitHub Release, so those plans still get an engine-created
    // Release (`coordinator-release-vs-cargo-dist-ownership`).
    let release_owner = github_release_owner(&targets);
    tag_phase(
        journal,
        sink,
        tagger,
        plan,
        &tag_commit,
        release_owner.as_deref(),
    )?;
    // dist (post-tag finalize): now the tag archive exists, finalize homebrew with
    // its real sha256. Runs for every cut (a no-op when there is no post-tag
    // target); its Ok completion flips the run to Completed. (`repo_slug` /
    // `homebrew` were moved into `artifacts` above; re-read them from there.)
    dist_phase(
        journal,
        sink,
        ctx,
        &targets,
        plan,
        artifacts.repo_slug.as_deref(),
        artifacts.homebrew.as_ref(),
    )?;
    verify_phase(
        journal,
        sink,
        ctx,
        &targets,
        plan,
        artifacts.homebrew.as_ref(),
    )?;

    Ok(())
}

/// Preflight a plan **without** touching external state or creating a run: check
/// it resolves into executable targets (every package resolved, no two targets
/// sharing a journal id).
///
/// `release cut` calls this *before* `Journal::create` so an unexecutable plan is
/// refused up front rather than leaving an orphaned `run_created` run behind.
/// [`execute`] re-runs the same resolution (defense in depth).
///
/// # Errors
/// [`CutError::Plan`] when a target has no resolved package, two *identical*
/// targets (same ecosystem, package, registry, and adapter) collide on one
/// journal id, or a Homebrew target has no servable platform.
pub fn validate_plan(plan: &ReleasePlan) -> Result<(), CutError> {
    resolve_target_plans(plan)?;
    if plan
        .targets
        .iter()
        .any(|target| matches!(target.adapter, Adapter::HomebrewTap | Adapter::HomebrewCore))
        && !plan.homebrew_platforms.iter().any(|triple| {
            crate::release::adapters::homebrew::homebrew_platform_condition(triple).is_some()
        })
    {
        return Err(CutError::Plan(
            "Homebrew formula has no Homebrew-servable cargo-dist platforms; supported platforms are macOS aarch64/x86_64 and Linux musl aarch64/x86_64; refusing to write a formula with no installable archive".into(),
        ));
    }
    Ok(())
}

/// Turn the sealed plan's abstract targets into concrete, adapter-backed units of
/// work — the one place a `null`-package or a duplicate target is refused (before
/// any external action).
///
/// Several targets in one ecosystem are supported (e.g. `ossctl-core` then
/// `ossctl` on crates.io): each is keyed by a distinct per-target journal id
/// ([`journal_target_ids`]), and the plan's (normalizer-canonical) order — which
/// lists a dependency before its dependents — is the publish order the barriers
/// walk. The coordinator alone owns cross-target ordering; the cargo adapter
/// publishes exactly its own target's crate and only *index-waits* on that crate's
/// workspace deps (ADR-0004, one target = one publish unit — no topo-sort, no
/// closure). The only collision left here is two byte-identical targets, a
/// degenerate contract duplicate.
fn resolve_target_plans(plan: &ReleasePlan) -> Result<Vec<TargetPlan>, CutError> {
    let ids = journal_target_ids(&plan.targets);
    let mut out = Vec::with_capacity(plan.targets.len());
    let mut seen: Vec<String> = Vec::new();
    for (t, id) in plan.targets.iter().zip(ids) {
        // A target whose package is still unresolved at cut time cannot publish —
        // the plan warned it would need inference; refuse rather than guess.
        let package = t.package.clone().ok_or_else(|| {
            CutError::Plan(format!(
                "target `{}` has no resolved package name — pin an explicit `package` \
                 in OSS-RELEASE.md and re-plan",
                t.ecosystem.as_str()
            ))
        })?;
        if seen.contains(&id) {
            return Err(CutError::Plan(format!(
                "two targets resolve to the same journal id `{id}` — the plan has two \
                 identical targets (same ecosystem, package, registry, and adapter); \
                 remove the duplicate target in OSS-RELEASE.md"
            )));
        }
        seen.push(id.clone());
        let input = AdapterTarget {
            target: Target {
                ecosystem: t.ecosystem,
                package: Some(package.clone()),
                registry: t.registry,
                adapter: t.adapter,
            },
            package,
            version: plan.version.clone(),
        };
        out.push(TargetPlan {
            id,
            adapter: resolve(t.adapter),
            input,
        });
    }
    Ok(out)
}

/// A throwaway, detached `git worktree` checked out at the plan's sealed commit —
/// the reproducible root every effect phase runs from (`release-cut-clean-checkout`).
///
/// [`materialize`](Self::materialize) creates it through the injected
/// [`CommandRunner`](crate::ports::CommandRunner) (no direct process effect in
/// `ossctl-core`) after **failing closed** if the sealed commit is not present
/// locally, so a cut can never publish from an unavailable or ambiguous tree. The
/// `Drop` impl removes the worktree on every *normal* exit path (success, phase
/// failure, or an unwinding panic), routed through the same runner, and follows the
/// removal with a best-effort `git worktree prune` to sweep any admin entry a
/// hard-killed prior run leaked. A hard kill (`SIGKILL`/`SIGTERM`, power loss,
/// `panic=abort`) skips `Drop` and leaves a `prune`-able stale entry at a unique
/// path — never wrong published output, and swept by the next cut's `prune`.
struct SealedCheckout<'a> {
    /// The runner the worktree add/remove shell out through (the effect seam).
    runner: &'a dyn CommandRunner,
    /// The **real** repository root — where the `git worktree add`/`remove` commands
    /// run (the worktree admin lives with the real repo, not inside the checkout).
    repo_root: &'a std::path::Path,
    /// The materialized checkout's path — the working directory every effect phase
    /// re-roots to, and the worktree `Drop` removes.
    path: std::path::PathBuf,
}

impl<'a> SealedCheckout<'a> {
    /// Materialize a clean detached checkout of `head_sha`, failing closed if that
    /// commit is not present in the local object store.
    ///
    /// Uses `ctx.repo_root` (the real repo) as the working directory for the git
    /// commands; the returned guard's [`path`](Self::path) is the checkout root the
    /// caller re-roots the effect context to.
    fn materialize(ctx: &EffectCtx<'a>, head_sha: &str) -> Result<SealedCheckout<'a>, CutError> {
        // Fail closed unless the sealed commit is present locally: `git cat-file -e
        // <sha>^{commit}` exits 0 only for a commit object that exists. A cut
        // publishes from THIS commit's bytes, so an absent/rewritten/gc'd commit is a
        // hard stop, not a fall-back-to-live-tree.
        let commitish = format!("{head_sha}^{{commit}}");
        let probe = ctx
            .runner
            .run("git", &["cat-file", "-e", &commitish], ctx.repo_root)
            .map_err(|e| {
                CutError::Checkout(format!(
                    "cannot probe the sealed commit `{head_sha}` (`git cat-file` failed to run: {e})"
                ))
            })?;
        if probe.status != Some(0) {
            return Err(CutError::Checkout(format!(
                "the sealed commit `{head_sha}` is not present in this repository (never \
                 committed, not fetched, or garbage-collected). Commit and push the release \
                 commit, then re-plan/cut — a cut publishes from a clean checkout of the sealed \
                 HEAD, never the live working tree"
            )));
        }

        let path = checkout_path(head_sha);
        let path_str = path.to_string_lossy().to_string();
        // `--detach` avoids creating a branch; the destination path is fresh (unique
        // per pid + nanos) so `git worktree add` never collides with a prior cut.
        let out = ctx
            .runner
            .run(
                "git",
                &["worktree", "add", "--detach", &path_str, head_sha],
                ctx.repo_root,
            )
            .map_err(|e| {
                CutError::Checkout(format!(
                    "cannot create a clean checkout worktree for `{head_sha}` \
                     (`git worktree add` failed to run: {e})"
                ))
            })?;
        if out.status != Some(0) {
            return Err(CutError::Checkout(format!(
                "`git worktree add` could not check out the sealed commit `{head_sha}` into a \
                 clean worktree at `{path_str}`: {}",
                out.stderr.trim()
            )));
        }

        Ok(SealedCheckout {
            runner: ctx.runner,
            repo_root: ctx.repo_root,
            path,
        })
    }

    /// The checkout root — the working directory the coordinator re-roots the effect
    /// context to for every phase.
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for SealedCheckout<'_> {
    fn drop(&mut self) {
        // Best-effort teardown on every normal exit path (success, phase failure,
        // unwinding panic). Routed through the runner so the coordinator performs no
        // direct process effect; `--force` also drops the worktree even if a leg
        // dirtied it (e.g. a build wrote target/). A failed removal only leaves a
        // prunable stale worktree — never affects what was published.
        let path_str = self.path.to_string_lossy().to_string();
        let _ = self.runner.run(
            "git",
            &["worktree", "remove", "--force", &path_str],
            self.repo_root,
        );
        // Sweep any admin entry left dangling — both this removal's (if `remove`
        // failed because the OS/tmp-reaper already deleted the directory) and any a
        // hard-killed PRIOR cut leaked (whose `Drop` never ran). `prune` only drops
        // entries whose worktree directory is gone and unlocked, so it can never
        // touch a valid user worktree; the single-active-cut flock means at most one
        // ossctl cut worktree is live at a time.
        let _ = self
            .runner
            .run("git", &["worktree", "prune"], self.repo_root);
    }
}

/// A fresh, unpredictable path for the sealed-commit checkout worktree — unique per
/// cut (pid + a nanosecond stamp + a short sha prefix) so concurrent cuts/tests
/// never collide and a crashed prior cut's leftover never blocks `git worktree add`.
/// Computing the path is not a filesystem effect; `git worktree add` (through the
/// runner) is what creates the directory.
fn checkout_path(head_sha: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let short: String = head_sha.chars().take(12).collect();
    std::env::temp_dir().join(format!("ossctl-cut-{}-{short}-{nanos}", std::process::id()))
}

/// The adapter identity of the target whose tag-triggered CI **owns the shared
/// GitHub Release** (via [`ci_owns_github_release`](ReleaseAdapter::ci_owns_github_release)),
/// or `None` when the coordinator owns it. `Some(_)` makes the tag phase delegate
/// Release creation to CI instead of creating it
/// (`coordinator-release-vs-cargo-dist-ownership`).
///
/// Returns the **first** such target's adapter (there is at most one in practice —
/// only `cargo-dist` claims Release ownership); the identity is journalled on the
/// delegation fact for the operator-facing record.
fn github_release_owner(targets: &[TargetPlan]) -> Option<String> {
    targets
        .iter()
        .find(|tp| tp.adapter.ci_owns_github_release())
        .map(|tp| tp.input.target.adapter.as_str().to_string())
}

/// Whether the cut carries a GitHub-backed distribution target — the binary
/// (`manual`, GitHub Releases) or a homebrew formula, both of which need the
/// repo's `origin` slug threaded into publish.
fn needs_github_slug(targets: &[TargetPlan]) -> bool {
    targets.iter().any(|tp| {
        matches!(
            tp.input.target.adapter,
            Adapter::Manual | Adapter::HomebrewTap | Adapter::HomebrewCore
        )
    })
}

/// Resolve the repo's `owner/repo` GitHub slug from its `origin` remote — the
/// input the two GitHub-backed distribution adapters need (binary's receipt URL,
/// homebrew's source-tarball URL + sha256).
///
/// Only shells out when a target actually consumes it (a binary or homebrew
/// target is in the cut); other cuts never touch git here. `None` when there is
/// no resolvable GitHub remote — a non-GitHub repo simply threads no slug (each
/// consumer then degrades honestly: binary records no `remote_url`, homebrew
/// threads no tarball).
fn resolve_repo_slug(ctx: &EffectCtx<'_>, targets: &[TargetPlan]) -> Option<String> {
    if !needs_github_slug(targets) {
        return None;
    }
    let out = ctx
        .runner
        .run("git", &["remote", "get-url", "origin"], ctx.repo_root)
        .ok()?;
    if out.status != Some(0) {
        return None;
    }
    crate::vcs::parse_github_slug(out.stdout.trim())
}

/// Resolve the cut's source tarball URL for the **pre-tag** phases (dry-run /
/// build preview) — the input a downstream Homebrew formula bump previews (`--url`).
///
/// Only produced when a homebrew target is in the cut; other cuts thread no
/// tarball. The `url` is the deterministic GitHub tag-archive URL for the plan's
/// tag (matching [`tag_archive_url`]).
///
/// # Why the pre-tag `sha256` is `None` (and where the real one is computed)
///
/// A Homebrew `--sha256` must be the hash of the exact bytes `--url` serves —
/// GitHub's tag archive — which **does not exist during dry-run / build**: the tag
/// is pushed in the coordinator-owned tag-once phase, *after* publish-all
/// (ADR-0002 §2), so there is nothing to fetch yet. A local `git archive` of the
/// same tree is **not** a substitute (its gzip framing diverges from GitHub's
/// served tarball, so the digest would be wrong), so the pre-tag preview threads
/// `sha256: None`.
///
/// The **real** digest is computed by the post-tag [`dist_phase`], which fetches
/// the pushed archive and hashes it ([`compute_source_tarball_sha256`]) before
/// finalizing the formula — so a homebrew cut no longer opens a draft PR with a
/// hand-filled hash (`release-engine-cut-cargo-dist-flow`).
fn source_tarball(slug: &str, plan: &ReleasePlan, targets: &[TargetPlan]) -> Option<SourceTarball> {
    let needed = targets.iter().any(|tp| {
        matches!(
            tp.input.target.adapter,
            Adapter::HomebrewTap | Adapter::HomebrewCore
        )
    });
    if !needed {
        return None;
    }
    let tag = format!("v{}", plan.version);
    Some(SourceTarball {
        url: format!("https://github.com/{slug}/archive/refs/tags/{tag}.tar.gz"),
        sha256: None,
    })
}

/// Resolve the Homebrew formula inputs — the destination tap + license — the
/// [`homebrew`](super::adapters::homebrew) adapter's first-formula bootstrap
/// needs beyond the source-tarball URL.
///
/// Only produced when a homebrew target is in the cut; other cuts thread `None`.
/// Both values are carried on the (already content-addressed) plan, copied there
/// from the normalized contract, so this is a pure re-projection — no external
/// state, no re-reading the contract.
fn homebrew_inputs(plan: &ReleasePlan, targets: &[TargetPlan]) -> Option<HomebrewFormula> {
    let needed = targets.iter().any(|tp| {
        matches!(
            tp.input.target.adapter,
            Adapter::HomebrewTap | Adapter::HomebrewCore
        )
    });
    if !needed {
        return None;
    }
    Some(HomebrewFormula {
        tap: plan.homebrew_tap.clone(),
        license: plan.license.clone(),
        description: plan.description.clone(),
        version: plan.version.clone(),
        platforms: plan.homebrew_platforms.clone(),
    })
}

/// Run a re-runnable phase (`dry_run` or `build`) as a strict barrier: every
/// target clears it (or is already recorded as cleared) before the phase
/// completes `Ok`; the first failure records `phase_completed … failed` and stops.
///
/// For the build phase `assets` accumulates each target's built artifact paths
/// (`Some` sink), so the coordinator can thread them into publish; the dry-run
/// phase passes `None`. A target skipped by resume contributes nothing — its
/// artifacts were gathered on the run that first built it.
fn reversible_phase(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    ctx: &EffectCtx<'_>,
    phase: Phase,
    targets: &[TargetPlan],
    mut assets: Option<&mut Vec<String>>,
) -> Result<(), CutError> {
    // Resume-readiness: a phase already completed Ok is skipped whole.
    if phase_completed_ok(journal.state(), phase) {
        return Ok(());
    }
    record(journal, sink, EventKind::PhaseEntered { phase })?;
    for tp in targets {
        // Skip a target already recorded as having cleared this phase.
        if target_cleared(journal.state(), phase, &tp.id) {
            continue;
        }
        let outcome = match phase {
            Phase::DryRun => tp.adapter.dry_run(ctx, &tp.input).map(|_| ()),
            Phase::Build => tp.adapter.build(ctx, &tp.input).map(|built| {
                if let Some(sink) = assets.as_deref_mut() {
                    sink.extend(built.artifacts);
                }
            }),
            Phase::Bump | Phase::Publish | Phase::Tag | Phase::Dist | Phase::Verify => {
                unreachable!("reversible_phase only runs dry_run/build")
            }
        };
        match outcome {
            Ok(()) => {
                let ev = match phase {
                    Phase::DryRun => EventKind::TargetDryRun {
                        target: tp.id.clone(),
                    },
                    Phase::Build => EventKind::TargetBuilt {
                        target: tp.id.clone(),
                    },
                    _ => unreachable!(),
                };
                record(journal, sink, ev)?;
            }
            Err(e) => return fail_phase(journal, sink, phase, Some(tp.id.clone()), e.to_string()),
        }
    }
    record(
        journal,
        sink,
        EventKind::PhaseCompleted {
            phase,
            outcome: PhaseOutcome::Ok,
        },
    )?;
    Ok(())
}

/// Run the publish-all barrier: each engine-owned target's `publish` is per-target
/// irreversible, so its receipt is journalled **immediately, before the next
/// target is attempted** (ADR-0003 §2 — never batched). The first failure records
/// `phase_completed publish failed` and stops with **no rollback** of what already
/// landed.
///
/// Two target classes are **not** published here:
/// - **CI-delegated** targets ([`is_ci_delegated`](ReleaseAdapter::is_ci_delegated)
///   — `cargo-dist` et al.) are journalled `target_delegated` and skipped: their
///   artifact is produced by the tag-triggered CI, so publishing from this host is
///   impossible, and treating the adapter's honest
///   [`AdapterError::Unsupported`](super::adapters::AdapterError::Unsupported) as a
///   failure would wedge the run after an irreversible crates.io publish. The
///   coordinator branches on the declared capability, **never** by catching
///   `Unsupported` (a genuine `Unsupported` from a non-delegated adapter still
///   fails the cut).
/// - **Post-tag** targets ([`needs_post_tag`] — homebrew) are deferred to the
///   [`dist_phase`], which runs after tag-once so the tag archive its formula
///   points at actually exists (a correct `sha256` cannot be computed before then).
fn publish_phase(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    ctx: &EffectCtx<'_>,
    targets: &[TargetPlan],
) -> Result<(), CutError> {
    let phase = Phase::Publish;
    if phase_completed_ok(journal.state(), phase) {
        return Ok(());
    }
    record(journal, sink, EventKind::PhaseEntered { phase })?;
    for tp in targets {
        // An already-published target (from a prior attempt) is never re-published.
        if journal.state().published.contains_key(&tp.id) {
            continue;
        }
        // Post-tag targets (homebrew) are finalized in the dist phase, not here —
        // their tarball only exists after the tag is pushed.
        if needs_post_tag(tp) {
            continue;
        }
        // A CI-delegated target already journalled `target_delegated` (a prior
        // attempt) is not re-journalled.
        if journal.state().delegated.contains(&tp.id) {
            continue;
        }
        // CI-delegated target: the tag-triggered CI produces its artifact, not the
        // engine. Journal the delegation and skip — do NOT publish, do NOT fail.
        if tp.adapter.is_ci_delegated() {
            record(
                journal,
                sink,
                EventKind::TargetDelegated {
                    target: tp.id.clone(),
                    adapter: tp.input.target.adapter.as_str().to_string(),
                },
            )?;
            continue;
        }
        match tp.adapter.publish(ctx, &tp.input) {
            Ok(receipt) => {
                record(
                    journal,
                    sink,
                    EventKind::TargetPublished {
                        target: tp.id.clone(),
                        receipt: to_journal_receipt(&receipt),
                    },
                )?;
            }
            Err(e) => return fail_phase(journal, sink, phase, Some(tp.id.clone()), e.to_string()),
        }
    }
    record(
        journal,
        sink,
        EventKind::PhaseCompleted {
            phase,
            outcome: PhaseOutcome::Ok,
        },
    )?;
    Ok(())
}

/// Run the engine-owned version-bump barrier (`--bump` plans only) — FIRST, before
/// dry-run-all, so every later phase builds and publishes the bumped tree
/// (`release-rust-workspace-multicrate` facet 2/3).
///
/// Applies the sealed edit set inside the clean checkout (version, `=`-pins, Cargo.lock,
/// CHANGELOG), runs any declared `bump_hook`, commits, and journals the resulting bump
/// commit as [`EventKind::BumpApplied`]. A no-bump plan is a clean no-op (returns before
/// entering the barrier). **Idempotent on resume:** a re-entered phase that already
/// carries the `BumpApplied` fact skips the (destructive) re-apply and just completes the
/// barrier — never double-bumps — while a phase interrupted *before* `BumpApplied` re-runs
/// the apply from the freshly re-materialized clean checkout (safe: the checkout is the
/// pristine sealed tree each cut). A failed apply records `phase_completed bump failed`
/// and stops before any build/publish — nothing external has happened.
fn bump_phase(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    ctx: &EffectCtx<'_>,
    plan: &ReleasePlan,
) -> Result<(), CutError> {
    let Some(bump) = plan.bump.as_ref() else {
        return Ok(());
    };
    let phase = Phase::Bump;
    if phase_completed_ok(journal.state(), phase) {
        return Ok(());
    }
    record(journal, sink, EventKind::PhaseEntered { phase })?;

    // Idempotent re-entry: if a prior attempt already applied + journalled the bump (an
    // interruption between `BumpApplied` and `phase_completed`), do NOT re-apply — the
    // commit already exists (the caller materialized the checkout AT it) and re-running the
    // edits/hook would be a double-bump. Only complete the barrier. Otherwise apply the
    // bump against the pristine checkout.
    if journal.state().bump.is_none() {
        let effective_date = crate::release::bump_exec::civil_date(ctx.clock.now_unix());
        match crate::release::bump_exec::apply_bump(ctx, bump, &effective_date) {
            Ok(outcome) => {
                record(
                    journal,
                    sink,
                    EventKind::BumpApplied {
                        commit: outcome.commit,
                        effective_date: outcome.effective_date,
                    },
                )?;
            }
            Err(e) => return fail_phase(journal, sink, phase, None, e.to_string()),
        }
    }

    record(
        journal,
        sink,
        EventKind::PhaseCompleted {
            phase,
            outcome: PhaseOutcome::Ok,
        },
    )?;
    Ok(())
}

/// Run the tag-once barrier — coordinator-owned, reached only after every publish
/// succeeded. Drives the three tag steps in order, each journalled separately and
/// each skipped if already recorded (resume). Any step failure records
/// `phase_completed tag failed` and stops, leaving completed steps journalled.
///
/// # GitHub Release ownership (`coordinator-release-vs-cargo-dist-ownership`)
///
/// The tag (`create_tag` → `push_tag`) is **always** created and pushed here —
/// that pushed tag is what triggers a CI-owned target's release workflow. The
/// third step, the GitHub Release, is conditional on `release_owner`:
///
/// - `None` (no target whose CI owns the Release): the coordinator creates the
///   Release itself through the injected [`Tagger`], exactly the ADR-0002 behavior,
///   journalling [`EventKind::GithubReleaseCreated`].
/// - `Some(adapter)` (a target whose CI owns the Release, e.g. `cargo-dist`): the
///   tag-triggered CI owns Release creation and the cross-platform binary upload, so
///   the coordinator does **not** create it — creating it first would clash with CI
///   (its `gh release create` then fails on "release already exists"). It records
///   [`EventKind::GithubReleaseDelegated`] (carrying `adapter`) instead, so
///   resume/verify treat the missing engine-created Release as intentional and a
///   resumed run never re-attempts it.
///
/// Either way exactly one Release-disposition fact is journalled per tag, and the
/// step is idempotent on resume (skipped once its fact is recorded). A
/// **contradictory** already-recorded disposition — a delegation demanded when the
/// journal already carries an engine-created Release, or vice versa — is refused as
/// a tag-phase failure rather than silently producing a dual-disposition state (it
/// is unreachable for a fixed `plan_id`, so it can only mean the adapter's ownership
/// classification changed under a resumed run's binary).
fn tag_phase(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    tagger: &dyn Tagger,
    plan: &ReleasePlan,
    tag_commit: &str,
    release_owner: Option<&str>,
) -> Result<(), CutError> {
    let phase = Phase::Tag;
    if phase_completed_ok(journal.state(), phase) {
        return Ok(());
    }
    record(journal, sink, EventKind::PhaseEntered { phase })?;

    let tag = format!("v{}", plan.version);
    let title = format!("Release {}", plan.version);

    if !tag_step_done(journal.state(), &tag, |s| s.created_local) {
        // Tag the run's landed commit: the engine-owned BUMP commit for a --bump run
        // (the bump advanced HEAD past the sealed pre-bump commit), else the plan's
        // sealed head_sha. Either way it is a fixed commit bound to the approval seam,
        // never "whatever HEAD is now".
        if let Err(e) = tagger.create_tag(&tag, tag_commit, &title) {
            return fail_phase(journal, sink, phase, None, format!("create local tag: {e}"));
        }
        record(
            journal,
            sink,
            EventKind::TagCreatedLocal { tag: tag.clone() },
        )?;
    }
    if !tag_step_done(journal.state(), &tag, |s| s.pushed_remote) {
        if let Err(e) = tagger.push_tag(&tag) {
            return fail_phase(journal, sink, phase, None, format!("push tag: {e}"));
        }
        record(
            journal,
            sink,
            EventKind::TagPushedRemote { tag: tag.clone() },
        )?;
    }
    // Refuse a contradictory already-recorded disposition before acting: the two
    // Release outcomes are mutually exclusive, so a delegation demanded over an
    // engine-created Release (or the reverse) is an invariant violation, not a step
    // to append on top of the other. Fail-and-journal, never a dual-disposition state.
    if let Some(adapter) = release_owner {
        if tag_step_done(journal.state(), &tag, |s| s.github_release) {
            return fail_phase(
                journal,
                sink,
                phase,
                None,
                format!(
                    "tag {tag} already has an engine-created GitHub Release, but the plan \
                     delegates the Release to CI ({adapter}); the adapter's ownership \
                     classification changed between attempts — reconcile the tag by hand"
                ),
            );
        }
    } else if tag_step_done(journal.state(), &tag, |s| s.github_release_delegated) {
        return fail_phase(
            journal,
            sink,
            phase,
            None,
            format!(
                "tag {tag}'s GitHub Release was already delegated to CI, but the plan now \
                 has the coordinator create it; the adapter's ownership classification \
                 changed between attempts — reconcile the tag by hand"
            ),
        );
    }

    if let Some(adapter) = release_owner {
        // A target's CI owns the GitHub Release: the tag pushed above triggers its
        // workflow, which creates+finalizes the Release and uploads the cross-platform
        // binaries. Record the delegation (skipped on resume once recorded) and do NOT
        // create the Release — creating it would clash with CI.
        if !tag_step_done(journal.state(), &tag, |s| s.github_release_delegated) {
            record(
                journal,
                sink,
                EventKind::GithubReleaseDelegated {
                    tag: tag.clone(),
                    delegated_to: adapter.to_string(),
                },
            )?;
        }
    } else if !tag_step_done(journal.state(), &tag, |s| s.github_release) {
        match tagger.create_github_release(&tag, &title) {
            Ok(url) => record(
                journal,
                sink,
                EventKind::GithubReleaseCreated {
                    tag: tag.clone(),
                    url,
                },
            )?,
            Err(e) => {
                return fail_phase(
                    journal,
                    sink,
                    phase,
                    None,
                    format!("create GitHub Release: {e}"),
                )
            }
        }
    }

    record(
        journal,
        sink,
        EventKind::PhaseCompleted {
            phase,
            outcome: PhaseOutcome::Ok,
        },
    )?;
    Ok(())
}

/// Whether a target is finalized in the **post-tag** dist phase rather than
/// publish-all: a homebrew formula, whose `url` is the tag archive that only exists
/// after tag-once, so a correct `sha256` cannot be computed until then.
fn needs_post_tag(tp: &TargetPlan) -> bool {
    matches!(
        tp.input.target.adapter,
        Adapter::HomebrewTap | Adapter::HomebrewCore
    )
}

/// Run the dist (post-tag finalize) barrier: finalize every post-tag target now
/// that the tag archive exists. For homebrew this resolves the pushed tag archive,
/// computes its **real** `sha256`, and hands it to the homebrew adapter so the
/// generated `.rb` (or `bump-formula-pr`) carries a correct hash — not the pre-tag
/// `sha256: None` draft-PR placeholder the publish phase could only produce.
///
/// Runs for every cut: one with no post-tag target enters and completes the barrier
/// as a clean no-op, so `dist ok` is the single, uniform completion signal. The
/// homebrew publish is per-target irreversible (it opens a PR), so its receipt is
/// journalled immediately and an already-published target (resume) is skipped. A
/// failure records `phase_completed dist failed` and stops — the tag already
/// landed, so this leaves an accurate, resumable record with no rollback.
fn dist_phase(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    ctx: &EffectCtx<'_>,
    targets: &[TargetPlan],
    plan: &ReleasePlan,
    repo_slug: Option<&str>,
    homebrew: Option<&HomebrewFormula>,
) -> Result<(), CutError> {
    let phase = Phase::Dist;
    if phase_completed_ok(journal.state(), phase) {
        return Ok(());
    }
    record(journal, sink, EventKind::PhaseEntered { phase })?;

    let post_tag: Vec<&TargetPlan> = targets.iter().filter(|tp| needs_post_tag(tp)).collect();
    if !post_tag.is_empty() {
        // Resolve the pushed tag archive and hash its exact bytes. Only possible
        // with a GitHub slug; without one the tarball is unresolvable and the
        // homebrew publish fails honestly below (its `source_tarball` is `None`).
        let source_tarball = match repo_slug {
            Some(slug) => {
                let url = tag_archive_url(slug, &plan.version);
                match compute_source_tarball_sha256(ctx, &url) {
                    Ok(sha256) => Some(SourceTarball {
                        url,
                        sha256: Some(sha256),
                    }),
                    Err(message) => return fail_phase(journal, sink, phase, None, message),
                }
            }
            None => None,
        };
        let homebrew_assets = match (repo_slug, homebrew) {
            (Some(slug), Some(formula)) => {
                match fetch_homebrew_assets(ctx, slug, plan, formula, &post_tag) {
                    Ok(assets) => assets,
                    Err(message) => return fail_phase(journal, sink, phase, None, message),
                }
            }
            _ => Vec::new(),
        };
        let artifacts = ReleaseArtifacts {
            assets: Vec::new(),
            source_tarball,
            repo_slug: repo_slug.map(str::to_string),
            homebrew: homebrew.cloned(),
            homebrew_assets,
        };
        let dist_ctx = ctx.with_artifacts(&artifacts);
        for tp in post_tag {
            // An already-finalized target (from a prior attempt) is never re-run.
            if journal.state().published.contains_key(&tp.id) {
                continue;
            }
            match tp.adapter.publish(&dist_ctx, &tp.input) {
                Ok(receipt) => {
                    record(
                        journal,
                        sink,
                        EventKind::TargetPublished {
                            target: tp.id.clone(),
                            receipt: to_journal_receipt(&receipt),
                        },
                    )?;
                }
                Err(e) => {
                    return fail_phase(journal, sink, phase, Some(tp.id.clone()), e.to_string())
                }
            }
        }
    }

    record(
        journal,
        sink,
        EventKind::PhaseCompleted {
            phase,
            outcome: PhaseOutcome::Ok,
        },
    )?;
    Ok(())
}

/// Maximum time the verify barrier waits for cargo-dist to create its delegated
/// GitHub Release and upload its cross-platform archives.
const DELEGATED_RELEASE_VERIFY_TIMEOUT_SECS: u64 = 20 * 60;
/// Delay between delegated GitHub Release observation attempts. Routed through
/// [`Clock::sleep`](crate::ports::Clock::sleep) so tests advance virtual time.
const DELEGATED_RELEASE_VERIFY_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(15);

/// Observe every destination after dist. A v5 cut is not complete until each
/// receipt or CI-delegation has an observed-good result; Unknown is deliberately
/// a barrier failure, never an implicit success.
fn verify_phase(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    ctx: &EffectCtx<'_>,
    targets: &[TargetPlan],
    plan: &ReleasePlan,
    homebrew: Option<&HomebrewFormula>,
) -> Result<(), CutError> {
    let phase = Phase::Verify;
    if phase_completed_ok(journal.state(), phase) {
        return Ok(());
    }
    let mut verification_artifacts = verification_artifacts(plan);
    verification_artifacts.homebrew = homebrew.cloned();
    let verify_ctx = ctx.with_artifacts(&verification_artifacts);
    record(journal, sink, EventKind::PhaseEntered { phase })?;
    for tp in targets {
        if journal.state().verified.get(&tp.id) == Some(&VerifyOutcome::Matches) {
            continue;
        }
        let outcome = if journal.state().delegated.contains(&tp.id) {
            match tp.input.target.registry {
                Registry::Homebrew => {
                    verify_delegated_homebrew(&verify_ctx, plan, &tp.input.package)
                }
                _ => verify_delegated_release(&verify_ctx, plan, &tp.input.package),
            }
        } else if let Some(receipt) = journal.state().published.get(&tp.id) {
            let receipt = AdapterReceipt {
                adapter: tp.input.target.adapter,
                ecosystem: tp.input.target.ecosystem,
                package: tp.input.package.clone(),
                version: receipt.version.clone(),
                canonical_ref: tp.input.canonical_ref(),
                digest: receipt.digest.clone(),
                remote_url: receipt.registry_url.clone(),
                timestamp: 0,
            };
            tp.adapter
                .verify(&verify_ctx, &receipt)
                .unwrap_or(VerifyOutcome::Unknown)
        } else {
            VerifyOutcome::Missing
        };
        record(
            journal,
            sink,
            EventKind::TargetVerified {
                target: tp.id.clone(),
                outcome,
            },
        )?;
        match outcome {
            VerifyOutcome::Matches => {}
            VerifyOutcome::Unknown => {
                return fail_phase(
                    journal,
                    sink,
                    phase,
                    Some(tp.id.clone()),
                    format!("could not observe {} at its destination", tp.id),
                )
            }
            VerifyOutcome::Missing => {
                return fail_phase(
                    journal,
                    sink,
                    phase,
                    Some(tp.id.clone()),
                    format!("{} is missing at its destination", tp.id),
                )
            }
            VerifyOutcome::Conflicts => {
                return fail_phase(
                    journal,
                    sink,
                    phase,
                    Some(tp.id.clone()),
                    format!("{} conflicts with its recorded receipt", tp.id),
                )
            }
        }
    }
    record(
        journal,
        sink,
        EventKind::PhaseCompleted {
            phase,
            outcome: PhaseOutcome::Ok,
        },
    )?;
    Ok(())
}

/// Observe a cargo-dist CI-owned Homebrew formula. Unlike the engine-owned tap
/// adapter, cargo-dist does not carry ossctl's ownership marker, but it must still
/// expose the sealed release version and expected platform archive stanzas.
fn verify_delegated_homebrew(
    ctx: &EffectCtx<'_>,
    plan: &ReleasePlan,
    package: &str,
) -> VerifyOutcome {
    let Some(tap) = plan.homebrew_tap.as_deref() else {
        debug_assert!(false, "delegated Homebrew target planned without a tap");
        return VerifyOutcome::Unknown;
    };
    let start = ctx.clock.now_unix();
    loop {
        let outcome = super::adapters::homebrew::verify_tap_formula(
            ctx,
            tap,
            package,
            &plan.version,
            false,
            Some(&plan.homebrew_platforms),
        );
        if outcome == VerifyOutcome::Matches
            || ctx.clock.now_unix().saturating_sub(start) >= DELEGATED_RELEASE_VERIFY_TIMEOUT_SECS
        {
            return outcome;
        }
        ctx.clock.sleep(DELEGATED_RELEASE_VERIFY_POLL_INTERVAL);
    }
}

/// Poll a cargo-dist owned GitHub Release until its expected platform archives are
/// visible. A failed `gh release view` is treated as not-yet-visible while within
/// the bounded window: CI can create the Release after the tag push.
fn verify_delegated_release(
    ctx: &EffectCtx<'_>,
    plan: &ReleasePlan,
    package: &str,
) -> VerifyOutcome {
    let expected: Vec<String> = plan
        .homebrew_platforms
        .iter()
        .map(|triple| format!("{package}-{triple}.tar.xz"))
        .collect();
    let start = ctx.clock.now_unix();
    loop {
        if observe_github_release_assets(ctx, &plan.version, &expected) == VerifyOutcome::Matches {
            return VerifyOutcome::Matches;
        }
        if ctx.clock.now_unix().saturating_sub(start) >= DELEGATED_RELEASE_VERIFY_TIMEOUT_SECS {
            return VerifyOutcome::Missing;
        }
        ctx.clock.sleep(DELEGATED_RELEASE_VERIFY_POLL_INTERVAL);
    }
}

/// The deterministic GitHub source-archive URL for `version`'s tag — the `url` a
/// downstream Homebrew formula points at, and the bytes whose `sha256` the dist
/// phase computes once the tag is pushed. Matches the pre-tag preview
/// [`source_tarball`] so the previewed and finalized `url` agree byte-for-byte.
fn tag_archive_url(slug: &str, version: &str) -> String {
    format!("https://github.com/{slug}/archive/refs/tags/v{version}.tar.gz")
}

/// How many times to (re)fetch the tag archive before giving up. GitHub's archive
/// endpoint is eventually consistent with a just-pushed tag — it can 404 for a few
/// seconds after `push_tag` — so a single fetch would spuriously fail the dist phase
/// on an otherwise-healthy cut.
const TAG_ARCHIVE_FETCH_ATTEMPTS: u32 = 5;

/// Backoff between tag-archive fetch attempts (through [`Clock::sleep`], so tests
/// advance a virtual clock rather than sleeping for real).
///
/// [`Clock::sleep`]: crate::ports::Clock::sleep
const TAG_ARCHIVE_FETCH_BACKOFF: std::time::Duration = std::time::Duration::from_secs(3);

/// Compute the `sha256` of the pushed tag archive at `url` by downloading and
/// hashing it through the injected [`CommandRunner`](crate::ports::CommandRunner)
/// — the coordinator never touches the network or filesystem directly.
///
/// Both effects go through the runner: `curl` streams the archive to a private,
/// unpredictable temp file (with a bounded retry, since the archive can be briefly
/// 404 right after the tag is pushed), then a SHA-256 CLI hashes it (its digest
/// lands on stdout, so a test fake supplies it deterministically and the coordinator
/// never reads the file itself). The temp file is removed on **every** exit path.
/// This hashes the EXACT bytes the formula's `url` serves — GitHub's tag archive for
/// the just-pushed tag — matching the working manual recipe; a local `git archive`
/// is deliberately NOT used (its gzip framing diverges from GitHub's served tarball,
/// so its digest would be wrong and `brew` would reject the download).
///
/// Returns the lowercase 64-hex digest, or an operator-facing error string when the
/// download/hash could not be performed or produced no usable digest.
fn compute_source_tarball_sha256(ctx: &EffectCtx<'_>, url: &str) -> Result<String, String> {
    let tmp = source_tarball_tmp_path();
    let tmp_str = tmp.to_string_lossy().to_string();
    let result = fetch_and_hash(ctx, url, &tmp_str);
    // Clean up on EVERY path (success or failure), routed through the runner so the
    // coordinator performs no direct filesystem effect. Its outcome is irrelevant.
    let _ = ctx.runner.run("rm", &["-f", &tmp_str], ctx.repo_root);
    result
}

/// Download the tag archive to `tmp` (with retry) then hash it. Split from
/// [`compute_source_tarball_sha256`] so the caller can guarantee temp-file cleanup
/// regardless of which step fails.
fn fetch_and_hash(ctx: &EffectCtx<'_>, url: &str, tmp: &str) -> Result<String, String> {
    fetch_tag_archive(ctx, url, tmp)?;
    hash_file(ctx, tmp)
}

/// Fetch `url` to `tmp` via `curl`, retrying a non-zero exit (a transient 404 on the
/// not-yet-consistent tag archive) up to [`TAG_ARCHIVE_FETCH_ATTEMPTS`] with backoff.
/// A spawn failure (`curl` absent) is fatal immediately — retrying cannot help.
/// `--` terminates option parsing so a `url` starting with `-` can never be read as
/// a flag.
fn fetch_tag_archive(ctx: &EffectCtx<'_>, url: &str, tmp: &str) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..TAG_ARCHIVE_FETCH_ATTEMPTS {
        let out = ctx
            .runner
            .run("curl", &["-sSfL", "-o", tmp, "--", url], ctx.repo_root)
            .map_err(|e| format!("cannot run `curl` to fetch the source tarball `{url}`: {e}"))?;
        if out.status == Some(0) {
            return Ok(());
        }
        last = format!(
            "exit {}: {}",
            out.status
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            out.stderr.trim()
        );
        if attempt + 1 < TAG_ARCHIVE_FETCH_ATTEMPTS {
            ctx.clock.sleep(TAG_ARCHIVE_FETCH_BACKOFF);
        }
    }
    Err(format!(
        "`curl` could not fetch the source tarball `{url}` after {TAG_ARCHIVE_FETCH_ATTEMPTS} \
         attempts ({last}); the tag archive may not be published yet"
    ))
}

/// A fresh, unpredictable temp path for the downloaded source tarball — unique per
/// attempt (pid + a nanosecond stamp) so concurrent cuts/tests never collide and a
/// retry never trips over a prior attempt's file. Computing the path is not a
/// filesystem effect; `curl` (through the runner) is what creates the file.
/// Wall-clock ceiling and poll interval for cargo-dist's asynchronously-uploaded
/// GitHub Release archives. These deliberately match the crates.io index wait: a
/// release cut is bounded and a missing artifact is a hard failure, never a source
/// build fallback.
const RELEASE_ASSET_WAIT_TIMEOUT_SECS: u64 = 300;
const RELEASE_ASSET_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// Fetch and hash every archive the generated Homebrew formula will serve. cargo-dist
/// starts only after the coordinator pushes the tag, so this runs in the post-tag dist
/// phase and waits for the exact assets rather than racing CI or writing placeholders.
fn fetch_homebrew_assets(
    ctx: &EffectCtx<'_>,
    slug: &str,
    plan: &ReleasePlan,
    formula: &HomebrewFormula,
    targets: &[&TargetPlan],
) -> Result<Vec<HomebrewAsset>, String> {
    let package = targets
        .first()
        .ok_or_else(|| "homebrew dist has no target".to_string())?
        .input
        .package
        .as_str();
    let mut assets = Vec::new();
    for triple in formula.platforms.iter().filter(|triple| {
        crate::release::adapters::homebrew::homebrew_platform_condition(triple).is_some()
    }) {
        let filename = format!("{package}-{triple}.tar.xz");
        let url = format!(
            "https://github.com/{slug}/releases/download/v{}/{filename}",
            plan.version
        );
        let tmp =
            std::env::temp_dir().join(format!("ossctl-homebrew-{filename}-{}", std::process::id()));
        let tmp_str = tmp.to_string_lossy().to_string();
        let start = ctx.clock.now_unix();
        #[allow(unused_assignments)]
        let mut last = String::new();
        let sha256_result = loop {
            let out = ctx.runner.run("curl", &["-sSfL", "-o", &tmp_str, "--", &url], ctx.repo_root)
                .map_err(|e| format!("cannot run `curl` while waiting for Homebrew release asset `{filename}`: {e}"))?;
            if out.status == Some(0) {
                break hash_file(ctx, &tmp_str)
                    .map_err(|e| format!("cannot hash Homebrew release asset `{filename}`: {e}"));
            }
            last = format!(
                "exit {}: {}",
                out.status
                    .map_or_else(|| "signal".to_string(), |c| c.to_string()),
                out.stderr.trim()
            );
            let waited = ctx.clock.now_unix().saturating_sub(start);
            if waited >= RELEASE_ASSET_WAIT_TIMEOUT_SECS {
                break Err(format!("Homebrew release asset `{filename}` was not visible after {waited}s (bounded release-asset wait; cargo-dist CI may have failed or not uploaded it): {last}. Refusing to write a source-build or unchecked formula"));
            }
            ctx.clock.sleep(RELEASE_ASSET_POLL_INTERVAL);
        };
        let _ = ctx.runner.run("rm", &["-f", &tmp_str], ctx.repo_root);
        let sha256 = sha256_result?;
        assets.push(HomebrewAsset {
            triple: triple.clone(),
            url,
            sha256,
        });
    }
    if assets.is_empty() {
        return Err("Homebrew formula has no Homebrew-servable cargo-dist platforms; supported platforms are macOS aarch64/x86_64 and Linux musl aarch64/x86_64; refusing to write a formula with no installable archive".to_string());
    }
    Ok(assets)
}

fn source_tarball_tmp_path() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!(
        "ossctl-src-tarball-{}-{nanos}.tar.gz",
        std::process::id()
    ))
}

/// Journal `phase_completed { phase, failed }` and return the [`CutError`] — the
/// single "stop and journal, never roll back" exit every phase failure funnels
/// through. If even the failure-record cannot be written, that journal error wins
/// (it is the more fundamental problem).
fn fail_phase(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    phase: Phase,
    target: Option<String>,
    message: String,
) -> Result<(), CutError> {
    record(
        journal,
        sink,
        EventKind::PhaseCompleted {
            phase,
            outcome: PhaseOutcome::Failed,
        },
    )?;
    Err(CutError::PhaseFailed {
        phase,
        target,
        message,
    })
}

/// Append `kind` to the journal (append-then-apply) and mirror the resulting
/// event to `sink`. The event handed to `sink` is reconstructed from the applied
/// state's watermark (`applied_seq`/`updated_ts`) so streaming never invents a
/// `seq`/`ts` the durable log does not carry.
fn record(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    kind: EventKind,
) -> Result<(), CutError> {
    let idempotency_key = kind.idempotency_key();
    let kind_for_sink = kind.clone();
    let state = journal.append(kind).map_err(CutError::Journal)?;
    let event = JournalEvent {
        schema_version: JOURNAL_SCHEMA_VERSION,
        seq: state.applied_seq,
        ts: state.updated_ts,
        idempotency_key,
        kind: kind_for_sink,
    };
    sink.event(&event);
    Ok(())
}

/// Whether `phase`'s barrier is already recorded as completed `Ok`.
fn phase_completed_ok(state: &RunState, phase: Phase) -> bool {
    state
        .phases
        .iter()
        .any(|r| r.phase == phase && r.outcome == PhaseOutcome::Ok)
}

/// Whether `target` is already recorded as having cleared `phase` (dry-run or
/// build) — the per-target resume skip.
fn target_cleared(state: &RunState, phase: Phase, target: &str) -> bool {
    match phase {
        Phase::DryRun => state.dry_run.contains(target),
        Phase::Build => state.built.contains(target),
        Phase::Publish | Phase::Dist => state.published.contains_key(target),
        Phase::Verify => state.verified.get(target) == Some(&VerifyOutcome::Matches),
        Phase::Bump | Phase::Tag => false,
    }
}

/// Whether a given tag landing-step (via `pick`) is already recorded for `tag`.
fn tag_step_done(
    state: &RunState,
    tag: &str,
    pick: impl Fn(&crate::protocol::journal::TagState) -> bool,
) -> bool {
    state.tags.get(tag).is_some_and(pick)
}

/// Project an adapter's rich [`AdapterReceipt`] onto the leaner
/// [`JournalReceipt`] the journal persists (the journal owns its own receipt
/// shape, ADR-0003). The canonical ref, adapter identity, and publish timestamp
/// are dropped — the journal already carries the target key and the event `ts`.
fn to_journal_receipt(r: &AdapterReceipt) -> JournalReceipt {
    JournalReceipt {
        ecosystem: r.ecosystem.as_str().to_string(),
        package: Some(r.package.clone()),
        version: r.version.clone(),
        registry_url: r.remote_url.clone(),
        digest: r.digest.clone(),
    }
}

#[cfg(test)]
mod tests;
