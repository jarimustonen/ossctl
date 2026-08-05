//! Phase-barrier coordinator: ordering, phase barriers, and tag ownership
//! (ADR-0002 §2).
//!
//! Drives every configured ecosystem adapter through the five barriers
//! **dry-run-all → build-all → publish-all → tag-once → dist (post-tag
//! finalize)**, with tagging owned by the coordinator alone (never an adapter).
//! This is the one stateful,
//! partially-irreversible operation in `ossctl`; the guarantees it enforces are:
//!
//! - **Strict barriers.** Every target must clear a phase before *any* target
//!   enters the next. A publish can never precede an all-targets build; a tag can
//!   never precede an all-targets publish. A failure in phase *K* blocks entry to
//!   *K+1* and records a `phase_completed { phase, outcome: failed }` fact.
//! - **Coordinator-only tagging.** The shared git tag + GitHub Release are
//!   created here, exactly once, only after every publish has succeeded, through
//!   the injected [`Tagger`] port. The three tag steps
//!   (`tag_created_local` → `tag_pushed_remote` → `github_release_created`) are
//!   independently journalled so an interrupted tag phase resumes step-by-step.
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
//! phase_entered tag     ; tag_created_local ; tag_pushed_remote ; github_release_created ; phase_completed tag ok
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

use crate::ports::Tagger;
use crate::protocol::journal::{
    EventKind, JournalEvent, Phase, PhaseOutcome, PublishReceipt as JournalReceipt, RunState,
    JOURNAL_SCHEMA_VERSION,
};
use crate::protocol::plan::ReleasePlan;
use crate::protocol::release::PublishReceipt as AdapterReceipt;

use super::adapters::{
    resolve, AdapterTarget, EcosystemAdapter, EffectCtx, HomebrewFormula, ReleaseAdapter,
    ReleaseArtifacts, SourceTarball,
};
use super::journal::Journal;
use super::journal_target_ids;
use crate::contract::schema::{Adapter, Target};

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
/// # Errors
/// Returns [`CutError`] on the first phase failure (barrier blocked), a journal
/// write failure, or an unexecutable plan. On a [`CutError::PhaseFailed`] the
/// partial state is already durably journalled — **nothing is rolled back**.
pub fn execute(
    journal: &mut Journal<'_>,
    plan: &ReleasePlan,
    ctx: &EffectCtx<'_>,
    tagger: &dyn Tagger,
    sink: &mut dyn ProgressSink,
) -> Result<(), CutError> {
    let targets = resolve_target_plans(plan)?;

    // Resolve the GitHub slug, source-tarball URL, and homebrew formula inputs up
    // front — they depend only on the plan + `origin` remote, never on any build
    // output — so the dry-run and build phases preview the *real*, fully
    // parameterized commands (the homebrew adapter needs the tap to even decide
    // create-vs-bump). Only `assets` (the binary upload set) is build-produced, so
    // it is empty for these pre-build phases and accumulated during build-all.
    let repo_slug = resolve_repo_slug(ctx, &targets);
    let source_tarball = repo_slug
        .as_deref()
        .and_then(|slug| source_tarball(slug, plan, &targets));
    let homebrew = homebrew_inputs(plan, &targets);
    let pre_artifacts = ReleaseArtifacts {
        assets: Vec::new(),
        source_tarball: source_tarball.clone(),
        repo_slug: repo_slug.clone(),
        homebrew: homebrew.clone(),
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
    };
    // publish-all: per-target irreversible; receipts journalled per target. The
    // publish phase is the only one that sees the build-complete artifacts. It
    // publishes the engine-owned targets, journals CI-delegated targets as skipped,
    // and defers post-tag targets (homebrew) to the dist phase below.
    publish_phase(journal, sink, &ctx.with_artifacts(&artifacts), &targets)?;
    // tag-once: coordinator-only, only after every publish succeeded.
    tag_phase(journal, sink, tagger, plan)?;
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
/// [`CutError::Plan`] when a target has no resolved package or two *identical*
/// targets (same ecosystem, package, registry, and adapter) collide on one
/// journal id.
pub fn validate_plan(plan: &ReleasePlan) -> Result<(), CutError> {
    resolve_target_plans(plan).map(|_| ())
}

/// Turn the sealed plan's abstract targets into concrete, adapter-backed units of
/// work — the one place a `null`-package or a duplicate target is refused (before
/// any external action).
///
/// Several targets in one ecosystem are supported (e.g. `ossctl-core` then
/// `ossctl` on crates.io): each is keyed by a distinct per-target journal id
/// ([`journal_target_ids`]), and the plan's (normalizer-canonical) order — which
/// lists a dependency before its dependents — is the publish order the barriers
/// walk. Intra-ecosystem dependency ordering *within* a single target (a
/// workspace's crates) is the adapter's own concern (the cargo adapter topo-sorts
/// and index-waits); the only collision left here is two byte-identical targets,
/// which is a degenerate contract duplicate.
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
            Phase::Publish | Phase::Tag | Phase::Dist => {
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

/// Run the tag-once barrier — coordinator-owned, reached only after every publish
/// succeeded. Drives the three tag steps in order, each journalled separately and
/// each skipped if already recorded (resume), then completes the phase `Ok`
/// (which flips the run to `Completed`). Any step failure records
/// `phase_completed tag failed` and stops, leaving completed steps journalled.
fn tag_phase(
    journal: &mut Journal<'_>,
    sink: &mut dyn ProgressSink,
    tagger: &dyn Tagger,
    plan: &ReleasePlan,
) -> Result<(), CutError> {
    let phase = Phase::Tag;
    if phase_completed_ok(journal.state(), phase) {
        return Ok(());
    }
    record(journal, sink, EventKind::PhaseEntered { phase })?;

    let tag = format!("v{}", plan.version);
    let title = format!("Release {}", plan.version);

    if !tag_step_done(journal.state(), &tag, |s| s.created_local) {
        // Tag the plan's SEALED commit, not whatever HEAD is now — the approval
        // seam binds HEAD, so the tag must point at the approved commit.
        if let Err(e) = tagger.create_tag(&tag, &plan.head_sha, &title) {
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
    if !tag_step_done(journal.state(), &tag, |s| s.github_release) {
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
        let artifacts = ReleaseArtifacts {
            assets: Vec::new(),
            source_tarball,
            repo_slug: repo_slug.map(str::to_string),
            homebrew: homebrew.cloned(),
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

/// The deterministic GitHub source-archive URL for `version`'s tag — the `url` a
/// downstream Homebrew formula points at, and the bytes whose `sha256` the dist
/// phase computes once the tag is pushed. Matches the pre-tag preview
/// [`source_tarball`] so the previewed and finalized `url` agree byte-for-byte.
fn tag_archive_url(slug: &str, version: &str) -> String {
    format!("https://github.com/{slug}/archive/refs/tags/v{version}.tar.gz")
}

/// Compute the `sha256` of the pushed tag archive at `url` by downloading and
/// hashing it through the injected [`CommandRunner`](crate::ports::CommandRunner)
/// — the coordinator never touches the network or filesystem directly.
///
/// Two commands, both through the runner: `curl` streams the archive to a private,
/// unpredictable temp file, then `shasum -a 256` hashes it (its digest lands on
/// stdout, so a test fake supplies it deterministically and the coordinator never
/// reads the file itself). The temp file is best-effort removed (again through the
/// runner). This hashes the EXACT bytes the formula's `url` serves — GitHub's tag
/// archive for the just-pushed tag — matching the working manual recipe; a local
/// `git archive` is deliberately NOT used (its gzip framing diverges from GitHub's
/// served tarball, so its digest would be wrong and `brew` would reject the
/// download).
///
/// Returns the lowercase 64-hex digest, or an operator-facing error string when the
/// download/hash could not be performed or produced no usable digest.
fn compute_source_tarball_sha256(ctx: &EffectCtx<'_>, url: &str) -> Result<String, String> {
    let tmp = source_tarball_tmp_path();
    let tmp_str = tmp.to_string_lossy().to_string();

    // 1. Download the tag archive to the temp file (`-f` fails on an HTTP error,
    //    `-L` follows GitHub's redirect to codeload).
    let dl = ctx
        .runner
        .run("curl", &["-sSfL", "-o", &tmp_str, url], ctx.repo_root)
        .map_err(|e| format!("cannot run `curl` to fetch the source tarball `{url}`: {e}"))?;
    if dl.status != Some(0) {
        return Err(format!(
            "`curl` could not fetch the source tarball `{url}` (exit {}): {}",
            dl.status
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            dl.stderr.trim()
        ));
    }

    // 2. Hash it. `shasum -a 256 <file>` prints `<hex>  <file>` on stdout.
    let out = ctx
        .runner
        .run("shasum", &["-a", "256", &tmp_str], ctx.repo_root)
        .map_err(|e| format!("cannot run `shasum` to hash the source tarball: {e}"))?;
    // Best-effort cleanup, routed through the runner so the coordinator performs no
    // direct filesystem effect. Its outcome is irrelevant to the digest.
    let _ = ctx.runner.run("rm", &["-f", &tmp_str], ctx.repo_root);
    if out.status != Some(0) {
        return Err(format!(
            "`shasum` could not hash the source tarball (exit {}): {}",
            out.status
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            out.stderr.trim()
        ));
    }
    let digest = out
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "could not parse a sha256 from `shasum` output: {:?}",
            out.stdout.trim()
        ));
    }
    Ok(digest)
}

/// A fresh, unpredictable temp path for the downloaded source tarball — unique per
/// attempt (pid + a nanosecond stamp) so concurrent cuts/tests never collide and a
/// retry never trips over a prior attempt's file. Computing the path is not a
/// filesystem effect; `curl` (through the runner) is what creates the file.
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
        Phase::Tag => false,
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
