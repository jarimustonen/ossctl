//! Phase-barrier coordinator: ordering, phase barriers, and tag ownership
//! (ADR-0002 §2).
//!
//! Drives every configured ecosystem adapter through the four barriers
//! **dry-run-all → build-all → publish-all → tag-once**, with tagging owned by
//! the coordinator alone (never an adapter). This is the one stateful,
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
//! phase_entered publish ; target_published …(receipt each, per target) ; phase_completed publish ok
//! phase_entered tag     ; tag_created_local ; tag_pushed_remote ; github_release_created ; phase_completed tag ok
//! ```
//!
//! `run_created` is written by [`Journal::create`] before [`execute`] runs; the
//! final `phase_completed tag ok` is what flips the run to
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
    // publish phase is the only one that sees the build-complete artifacts.
    publish_phase(journal, sink, &ctx.with_artifacts(&artifacts), &targets)?;
    // tag-once: coordinator-only, only after every publish succeeded.
    tag_phase(journal, sink, tagger, plan)?;

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

/// Resolve the cut's published source tarball URL — the input a downstream
/// Homebrew formula bump needs (`--url`).
///
/// Only produced when a homebrew target is in the cut; other cuts thread no
/// tarball. The `url` is the deterministic GitHub tag-archive URL for the plan's
/// tag.
///
/// # Why `sha256` is `None` (deliberate, not deferred work)
///
/// A Homebrew `--sha256` must be the hash of the exact bytes `--url` serves —
/// GitHub's on-the-fly tag archive. That archive **cannot be hashed correctly
/// here**:
///
/// - It does not exist yet: the tag is pushed in the coordinator-owned tag-once
///   phase, *after* publish-all (ADR-0002 §2), so there is nothing to fetch.
/// - A local `git archive` of the same tree is **not** a substitute: its gzip
///   framing (and `git`/libarchive version differences) make its bytes — and thus
///   its sha256 — diverge from GitHub's served tarball, whose checksum GitHub
///   explicitly does not guarantee to be stable. A plausible-but-wrong `--sha256`
///   is *worse* than none: `brew` (and Homebrew CI) reject the download on
///   mismatch, whereas omitting it lets `brew` compute the correct digest from
///   `--url` itself.
///
/// So this threads `sha256: None` and the formula bump omits `--sha256`. The
/// genuinely-correct digest needs the *pushed* archive (a post-tag distribution
/// phase) or an ossctl-built, ossctl-uploaded source-tarball asset whose bytes it
/// controls — both an ADR-0002 phase-model change tracked separately. See the
/// merge report's discussion items.
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
            Phase::Publish | Phase::Tag => unreachable!("reversible_phase only runs dry_run/build"),
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

/// Run the publish-all barrier: each target's `publish` is per-target
/// irreversible, so its receipt is journalled **immediately, before the next
/// target is attempted** (ADR-0003 §2 — never batched). The first failure records
/// `phase_completed publish failed` and stops with **no rollback** of what already
/// landed.
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
        Phase::Publish => state.published.contains_key(target),
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
