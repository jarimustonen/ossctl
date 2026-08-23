//! Event-sourced release journal (ADR-0003).
//!
//! Append-only JSONL events under `git-common-dir/ossctl/releases/<run_id>/`,
//! with an idempotent [`reduce`]r folding them into resumable [`RunState`]. The
//! durable record `release resume`/`verify`/`show` read back.
//!
//! # The two halves
//!
//! - **The reducer** ([`reduce`] / [`apply`]) is a *pure* fold of `[JournalEvent]
//!   → RunState`. It has no I/O and is the core testable unit: the same events
//!   always fold to the same state, and re-applying a seen event changes nothing.
//! - **The [`Journal`] handle** wires the reducer to durable storage through the
//!   injected [`JournalStore`] / [`Clock`] / [`IdGen`] ports, enforcing the
//!   append-then-apply atomicity discipline and holding the single-active-cut
//!   lock for its lifetime.
//!
//! # Append-then-apply (ADR-0003 §2, from `octl-core`)
//!
//! Every mutation is: **(1)** fsync the event to `journal.jsonl` (durable),
//! **(2)** apply it to the in-memory [`RunState`], **(3)** atomically rewrite the
//! `manifest.json` cache. The journal is the single source of truth; the manifest
//! is disposable and is always rebuilt by [`reduce`] on [`Journal::open`], so a
//! crash *anywhere* in that sequence recovers cleanly — a durably-appended event
//! is folded back in on the next open regardless of whether its manifest write
//! landed.
//!
//! # Idempotency
//!
//! Replay idempotency — the property the crash-safety discipline needs — comes
//! from two mechanisms working together:
//!
//! 1. **Watermark** — [`apply`] ignores any event whose `seq` is at or below the
//!    already-applied high-water mark, so replaying the persisted log (or a seen
//!    event) is a no-op. This is the "re-applying a seen event changes nothing"
//!    guarantee.
//! 2. **Structural** — the projection is built from keyed sets/maps, so folding a
//!    fact about target `cargo` more than once yields the identical map.
//!
//! The log is **append-only facts**: [`Journal::append`] does not deduplicate on
//! [`crate::protocol::journal::JournalEvent::idempotency_key`] (an earlier design
//! did, which silently swallowed a legitimate `Failed`→`Ok` phase retry after a
//! resume). Whether to *emit* an event is the coordinator's decision — and since
//! the remote registry is ground truth (ADR-0003 §4), a resumed cut re-checks
//! reality before re-emitting rather than trusting an append gate.
//!
//! # Terminal states
//!
//! Once the run reaches a terminal status ([`RunStatus::Completed`] or
//! [`RunStatus::Abandoned`]) the reducer freezes: later events are ignored, so a
//! corrupt or buggy log cannot un-abandon a run or resurrect a completed one.

use std::io;
use std::path::{Path, PathBuf};

use crate::ports::{Clock, GitRepo, IdGen, JournalLock, JournalStore};
use crate::protocol::journal::{
    EventKind, JournalEvent, Phase, PhaseOutcome, PhaseRecord, RunState, RunStatus,
    JOURNAL_SCHEMA_VERSION,
};

/// Resolved on-disk locations for a repo's release journals.
///
/// The releases root is `git-common-dir/ossctl/releases` (ADR-0003 §3), resolved
/// via [`GitRepo::git_common_dir`] — **never** by concatenating `.git/` — or an
/// explicit override for CI / debugging (`--journal-dir`). All per-run paths and
/// the single-active-cut lock path are derived from it.
#[derive(Debug, Clone)]
pub struct JournalPaths {
    releases_dir: PathBuf,
}

impl JournalPaths {
    /// Build paths rooted at an explicit `releases_dir` (the `--journal-dir`
    /// override, or a test root).
    pub fn new(releases_dir: impl Into<PathBuf>) -> Self {
        Self {
            releases_dir: releases_dir.into(),
        }
    }

    /// Resolve the releases root from git — `<git-common-dir>/ossctl/releases` —
    /// unless `override_dir` is supplied, in which case it is used verbatim.
    ///
    /// # Errors
    /// Propagates a [`GitRepo::git_common_dir`] failure (not a git repository, or
    /// git unavailable) when no override is given.
    pub fn from_git(git: &dyn GitRepo, override_dir: Option<&Path>) -> io::Result<Self> {
        let releases_dir = match override_dir {
            Some(dir) => dir.to_path_buf(),
            // COMPATIBILITY (ADR-0005 §3): renaming this component would split
            // the single-cut lock and orphan every stored plan and journal.
            None => git.git_common_dir()?.join("ossctl").join("releases"),
        };
        Ok(Self { releases_dir })
    }

    /// The releases root (`…/ossctl/releases`).
    #[must_use]
    pub fn releases_dir(&self) -> &Path {
        &self.releases_dir
    }

    /// The immutable plan-store root (`…/ossctl/plans`), a sibling of releases.
    #[must_use]
    pub fn plans_dir(&self) -> PathBuf {
        self.releases_dir
            .parent()
            .expect("release paths always have the legacy-compatible ossctl parent")
            .join("plans")
    }

    /// The content-addressed plan document for `plan_id`.
    #[must_use]
    pub fn plan_file(&self, plan_id: &str) -> PathBuf {
        self.plans_dir().join(format!("{plan_id}.json"))
    }

    /// Durable disposal marker used to distinguish an idempotent retry from an
    /// address that was never present.
    #[must_use]
    pub fn discarded_plan_file(&self, plan_id: &str) -> PathBuf {
        self.plans_dir()
            .join("discarded")
            .join(format!("{plan_id}.discarded"))
    }

    /// The single-active-cut lock path (`…/releases/.lock`).
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.releases_dir.join(".lock")
    }

    /// The per-run directory (`…/releases/<run_id>/`).
    #[must_use]
    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.releases_dir.join(run_id)
    }

    /// The append-only event log for a run (`…/<run_id>/journal.jsonl`).
    #[must_use]
    pub fn journal_file(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("journal.jsonl")
    }

    /// The materialized state cache for a run (`…/<run_id>/manifest.json`).
    #[must_use]
    pub fn manifest_file(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("manifest.json")
    }
}

// ── The reducer (pure) ───────────────────────────────────────────────────────

/// Fold an ordered event stream into the materialized [`RunState`] — the pure,
/// I/O-free core of the journal.
///
/// Deterministic and total: the same events (in `seq` order) always produce the
/// same state. Events are applied in ascending `seq` regardless of slice order,
/// so a defensively re-sorted log reduces identically.
#[must_use]
pub fn reduce(events: &[JournalEvent]) -> RunState {
    let mut ordered: Vec<&JournalEvent> = events.iter().collect();
    ordered.sort_by_key(|e| e.seq);
    let mut state = RunState::empty();
    for ev in ordered {
        apply(&mut state, ev);
    }
    state
}

/// Apply a single event to `state`, in place.
///
/// **Idempotent**: an event whose `seq` is at or below `state.applied_seq` is
/// skipped (the high-water mark), and every mutation targets a keyed set/map, so
/// re-applying a seen event leaves the state byte-identical. This is what makes
/// append-then-apply crash-safe — replaying after a crash is a clean
/// no-op-or-apply.
///
/// **Terminal-safe**: once the run is [`RunStatus::Completed`] or
/// [`RunStatus::Abandoned`], further events are ignored (the watermark still
/// advances so a later legitimate replay is consistent), so a corrupt log cannot
/// mutate a run past its terminal fact.
#[allow(clippy::too_many_lines)] // one exhaustive reducer keeps event semantics auditable
pub fn apply(state: &mut RunState, event: &JournalEvent) {
    // Watermark: never fold an event already accounted for. `seq` starts at 1,
    // so the first event (seq 1) always applies against the initial mark of 0.
    if event.seq <= state.applied_seq {
        return;
    }
    // Terminal states freeze the projection: nothing recorded after a run is
    // completed or abandoned may change it. Advance the watermark so the state
    // still reflects "everything up to here has been seen".
    if matches!(state.status, RunStatus::Completed | RunStatus::Abandoned) {
        state.applied_seq = event.seq;
        return;
    }
    match &event.kind {
        EventKind::RunCreated {
            run_id,
            plan_id,
            version,
            targets,
            head_sha,
            bump,
        } => {
            state.run_id.clone_from(run_id);
            state.plan_id.clone_from(plan_id);
            state.version.clone_from(version);
            state.targets.clone_from(targets);
            state.head_sha.clone_from(head_sha);
            state.bump_inputs.clone_from(bump);
            state.created_ts = event.ts;
            state.terminal_phase = Some(match event.schema_version {
                0 | 1 => Phase::Tag,
                2..=4 => Phase::Dist,
                5 => Phase::Verify,
                _ => Phase::AdvanceBranch,
            });
            state.status = RunStatus::InProgress;
        }
        EventKind::BumpApplied {
            commit,
            effective_date,
        } => {
            state.bump = Some(crate::protocol::journal::BumpRecord {
                commit: commit.clone(),
                effective_date: effective_date.clone(),
            });
        }
        EventKind::PhaseEntered { phase } => {
            state.current_phase = Some(*phase);
        }
        EventKind::PhaseCompleted { phase, outcome } => {
            upsert_phase(&mut state.phases, *phase, *outcome);
            if state.current_phase == Some(*phase) {
                state.current_phase = None;
            }
            // The final barrier completing OK is the run's completion signal. For a
            // v2 cut that is the post-tag Dist barrier — it runs after Tag for every
            // cut (a no-op when there is no post-tag target), so `Dist Ok` is the
            // single, uniform completion signal (ADR-0002 §2, extended by
            // `release-engine-cut-cargo-dist-flow`).
            //
            // A **v1** log has no Dist phase and ended at `Tag Ok`; that event
            // carries `schema_version < 2`, so it still completes the run. Without
            // this, a v1-completed run would reduce to InProgress — misreporting a
            // finished release and making the manifest cache (which the old reducer
            // wrote as Completed) disagree with a fresh reduce of the same log. A
            // v2 `Tag Ok` (schema_version >= 2) does NOT complete: a Dist barrier
            // always follows it, and completing early would freeze the projection
            // before Dist runs.
            let completes = state.terminal_phase == Some(*phase);
            if completes && *outcome == PhaseOutcome::Ok {
                state.status = RunStatus::Completed;
            }
        }
        EventKind::TargetDryRun { target } => {
            state.dry_run.insert(target.clone());
        }
        EventKind::TargetBuilt { target } => {
            state.built.insert(target.clone());
        }
        EventKind::TargetPublished { target, receipt } => {
            state.published.insert(target.clone(), receipt.clone());
        }
        EventKind::TargetCancelled { target, reason } => {
            state.cancelled.insert(target.clone(), reason.clone());
        }
        EventKind::TargetDelegated { target, adapter } => {
            state.delegated.insert(target.clone());
            state
                .delegated_adapters
                .insert(target.clone(), adapter.clone());
        }
        EventKind::TargetVerified { target, outcome } => {
            state.verified.insert(target.clone(), *outcome);
        }
        EventKind::TagCreatedLocal { tag } => {
            state.tags.entry(tag.clone()).or_default().created_local = true;
        }
        EventKind::TagPushedRemote { tag } => {
            state.tags.entry(tag.clone()).or_default().pushed_remote = true;
        }
        EventKind::DefaultBranchSelected { branch } => {
            state.selected_default_branch = Some(branch.clone());
        }
        EventKind::DefaultBranchAdvanced { branch, commit } => {
            state.default_branch = Some(crate::protocol::journal::DefaultBranchState {
                branch: branch.clone(),
                commit: commit.clone(),
            });
        }
        EventKind::GithubReleaseCreated { tag, url } => {
            let t = state.tags.entry(tag.clone()).or_default();
            t.github_release = true;
            t.github_release_url.clone_from(url);
        }
        EventKind::GithubReleaseDelegated { tag, .. } => {
            state
                .tags
                .entry(tag.clone())
                .or_default()
                .github_release_delegated = true;
        }
        EventKind::RunAbandoned { reason } => {
            state.status = RunStatus::Abandoned;
            state.abandon_reason = Some(reason.clone());
        }
    }
    state.applied_seq = event.seq;
    state.updated_ts = event.ts;
}

/// Insert-or-update a completed-phase record, keeping `phases` sorted by phase
/// order (so the manifest is deterministic).
fn upsert_phase(phases: &mut Vec<PhaseRecord>, phase: Phase, outcome: PhaseOutcome) {
    if let Some(rec) = phases.iter_mut().find(|r| r.phase == phase) {
        rec.outcome = outcome;
    } else {
        phases.push(PhaseRecord { phase, outcome });
        phases.sort_by_key(|r| r.phase);
    }
}

// ── Reading events back (with forward tolerance) ─────────────────────────────

/// Parse the JSONL journal at `path` into events, in ascending `seq`.
///
/// Forward-tolerant per ADR-0003 §2: additive fields are ignored (serde does not
/// `deny_unknown_fields`), but an event whose `schema_version` is **newer** than
/// this binary understands — or whose `kind` this binary does not know — is
/// refused with an actionable error rather than silently mutating state.
///
/// # Errors
/// [`io::ErrorKind::InvalidData`] on a malformed or too-new event line, or any
/// I/O error surfaced by the store.
pub fn read_events(store: &dyn JournalStore, path: &Path) -> io::Result<Vec<JournalEvent>> {
    /// A minimal envelope parsed *before* the strict [`JournalEvent`] so a newer
    /// schema (which may carry an unknown `kind` the enum cannot deserialize) is
    /// refused with an actionable upgrade error rather than a generic parse error.
    #[derive(serde::Deserialize)]
    struct Envelope {
        schema_version: u32,
    }

    let lines = store.read_lines(path)?;
    let mut events = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Version gate first: a too-new event is refused before the strict enum
        // parse (which would otherwise fail on an unknown `kind` with a generic
        // message and never reach this check).
        if let Ok(envelope) = serde_json::from_str::<Envelope>(trimmed) {
            if envelope.schema_version > JOURNAL_SCHEMA_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "release journal {}: line {} has schema_version {} but this \
                         shipshape understands at most {}; upgrade shipshape to resume this run",
                        path.display(),
                        idx + 1,
                        envelope.schema_version,
                        JOURNAL_SCHEMA_VERSION
                    ),
                ));
            }
        }
        let event: JournalEvent = serde_json::from_str(trimmed).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "release journal {}: line {} is not a recognized event \
                     (corrupt, or written by a newer shipshape): {e}",
                    path.display(),
                    idx + 1
                ),
            )
        })?;
        events.push(event);
    }
    events.sort_by_key(|e| e.seq);
    Ok(events)
}

/// Reject a `run_id` that is not a single safe path segment.
///
/// `run_id`s minted by `IdGen` are ULIDs, but `open`/`load_state` also take a
/// caller-supplied id (a CLI argument), so a `../…`, an absolute path, or an
/// empty string must never be joined into the releases root (path traversal).
fn validate_run_id(run_id: &str) -> io::Result<()> {
    let bad = run_id.is_empty()
        || run_id == "."
        || run_id == ".."
        || run_id.contains('/')
        || run_id.contains('\\')
        || run_id.contains('\0');
    if bad {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid run id {run_id:?}: must be a single path segment"),
        ));
    }
    Ok(())
}

/// Read a run's current state **without** locking — the read-only path behind
/// `release show`.
///
/// Prefers the atomically-written `manifest.json` cache (a torn-free read, and
/// O(1) versus replaying the log); falls back to reducing the authoritative
/// `journal.jsonl` when the manifest is absent, unparsable, or does not match the
/// run. The cache can lag the log by the last event after a crash between append
/// and manifest write, so this is *best-effort* for display — a
/// correctness-critical read (resume/reconcile) must go through the locked
/// [`Journal::open`], which always reduces the log.
///
/// Returns `Ok(None)` when the run has neither a usable manifest nor a journal.
///
/// # Errors
/// An invalid `run_id`, or any store/parse error from [`read_events`].
pub fn load_state(
    store: &dyn JournalStore,
    paths: &JournalPaths,
    run_id: &str,
) -> io::Result<Option<RunState>> {
    validate_run_id(run_id)?;
    // Fast path: the atomic manifest cache (never torn).
    if let Some(bytes) = store.read(&paths.manifest_file(run_id))? {
        if let Ok(state) = serde_json::from_slice::<RunState>(&bytes) {
            if state.run_id == run_id && state.schema_version <= JOURNAL_SCHEMA_VERSION {
                return Ok(Some(state));
            }
        }
        // A corrupt/mismatched cache falls through to the authoritative rebuild.
    }
    let events = read_events(store, &paths.journal_file(run_id))?;
    if events.is_empty() {
        return Ok(None);
    }
    Ok(Some(reduce(&events)))
}

/// Authoritatively rebuild a run's [`RunState`] straight from its event log —
/// the read-only path `release verify` reconciles from.
///
/// Unlike [`load_state`] this **ignores the `manifest.json` fast-path** and always
/// reduces the authoritative `journal.jsonl`, so a manifest that lags the log by
/// its last event (a crash between append and manifest write) cannot hide a
/// just-published receipt from the reconcile. Unlike [`Journal::open`] it takes
/// **no lock and writes nothing** — not even the manifest self-heal — so it is
/// safe against a live run and leaves the journal byte-for-byte unchanged (the
/// read-only guarantee `verify` promises).
///
/// Returns `Ok(None)` when the run has no journal.
///
/// # Errors
/// An invalid `run_id` ([`io::ErrorKind::InvalidInput`]), or any store/parse error
/// from [`read_events`] (a corrupt or too-new event line).
pub fn read_run_state(
    store: &dyn JournalStore,
    paths: &JournalPaths,
    run_id: &str,
) -> io::Result<Option<RunState>> {
    validate_run_id(run_id)?;
    let events = read_events(store, &paths.journal_file(run_id))?;
    if events.is_empty() {
        return Ok(None);
    }
    Ok(Some(reduce(&events)))
}

/// Read a run's event log **and** its reduced state, read-only — the read path
/// behind `release show`.
///
/// `release show` needs both halves: the ordered [`JournalEvent`] log (to stream
/// as a live JSONL event window) *and* the folded [`RunState`] (to decide live
/// vs. terminal and render the post-mortem summary). This is the write-free,
/// unlocked twin of [`Journal::open`] that returns the log alongside the
/// projection so the caller reduces once, not twice.
///
/// Like [`read_run_state`] it ignores the `manifest.json` fast path and reduces
/// the authoritative `journal.jsonl`, so a manifest lagging the log by its last
/// event cannot hide the newest fact from a live tail. Returns `Ok(None)` when
/// the run has no journal.
///
/// # Errors
/// An invalid `run_id` ([`io::ErrorKind::InvalidInput`]), or any store/parse
/// error from [`read_events`].
pub fn read_run(
    store: &dyn JournalStore,
    paths: &JournalPaths,
    run_id: &str,
) -> io::Result<Option<(Vec<JournalEvent>, RunState)>> {
    validate_run_id(run_id)?;
    let events = read_events(store, &paths.journal_file(run_id))?;
    if events.is_empty() {
        return Ok(None);
    }
    let state = reduce(&events);
    Ok(Some((events, state)))
}

/// List the run ids present under the releases root — the enumeration behind
/// `release list`.
///
/// Only entries that carry a non-empty `journal.jsonl` are returned, so the
/// `.lock` file, atomic-write temp files, and any stray files/dirs the store may
/// surface are excluded (a run is defined by having a journal).
///
/// # Errors
/// Any store error listing the releases directory.
pub fn list_runs(store: &dyn JournalStore, paths: &JournalPaths) -> io::Result<Vec<String>> {
    let mut runs: Vec<String> = store
        .list_dir(paths.releases_dir())?
        .into_iter()
        .filter(|name| validate_run_id(name).is_ok())
        .filter(|name| {
            // A real run has a non-empty journal; this also excludes temp files
            // and empty/aborted run directories.
            store
                .read_lines(&paths.journal_file(name))
                .is_ok_and(|lines| lines.iter().any(|l| !l.trim().is_empty()))
        })
        .collect();
    runs.sort();
    Ok(runs)
}

// ── The Journal handle (durable, locked) ─────────────────────────────────────

/// A live, exclusively-locked handle to one release run's journal.
///
/// Holds the single-active-cut lock for its entire lifetime (dropping the handle
/// releases it) and mediates every mutation through the append-then-apply
/// discipline. Construct it with [`Journal::create`] (a new run) or
/// [`Journal::open`] (resume an existing one).
pub struct Journal<'a> {
    store: &'a dyn JournalStore,
    clock: &'a dyn Clock,
    paths: JournalPaths,
    run_id: String,
    state: RunState,
    /// The single-active-cut lock, released on drop. Held, never called.
    _lock: Box<dyn JournalLock>,
}

impl<'a> Journal<'a> {
    /// Create a brand-new run: take the single-active-cut lock, mint a `run_id`
    /// via `idgen`, and record the `RunCreated` event (which is also persisted to
    /// a fresh manifest).
    ///
    /// # Errors
    /// [`io::ErrorKind::WouldBlock`] if another cut/resume holds the lock, plus
    /// any store error appending the first event or writing the manifest.
    pub fn create(
        store: &'a dyn JournalStore,
        clock: &'a dyn Clock,
        idgen: &dyn IdGen,
        paths: JournalPaths,
        plan_id: String,
        version: String,
        targets: Vec<String>,
    ) -> io::Result<Self> {
        Self::create_inner(
            store, clock, idgen, paths, plan_id, version, targets, None, None,
        )
    }

    /// Create a `--bump` run, persisting the sealed `head_sha` + [`BumpInputs`](crate::protocol::journal::BumpInputs) on the
    /// `RunCreated` event so `release resume` can reconstruct the exact sealed plan
    /// (`build_with_bump`) against the pre-bump commit after the bump commit moves HEAD
    /// (`release-rust-workspace-multicrate`). Otherwise identical to [`Self::create`].
    ///
    /// # Errors
    /// As [`Self::create`].
    #[allow(clippy::too_many_arguments)]
    pub fn create_bump(
        store: &'a dyn JournalStore,
        clock: &'a dyn Clock,
        idgen: &dyn IdGen,
        paths: JournalPaths,
        plan_id: String,
        version: String,
        targets: Vec<String>,
        head_sha: String,
        bump: crate::protocol::journal::BumpInputs,
    ) -> io::Result<Self> {
        Self::create_inner(
            store,
            clock,
            idgen,
            paths,
            plan_id,
            version,
            targets,
            Some(head_sha),
            Some(bump),
        )
    }

    /// Create a run using a release lock the caller already holds. This lets a
    /// caller authenticate a referenced sealed plan under the same lock before
    /// durably publishing the `RunCreated` reference.
    #[allow(clippy::too_many_arguments)]
    pub fn create_locked(
        store: &'a dyn JournalStore,
        clock: &'a dyn Clock,
        idgen: &dyn IdGen,
        paths: JournalPaths,
        plan_id: String,
        version: String,
        targets: Vec<String>,
        lock: Box<dyn JournalLock>,
    ) -> io::Result<Self> {
        Self::create_inner_locked(
            store, clock, idgen, paths, plan_id, version, targets, None, None, lock,
        )
    }

    /// Bump-aware counterpart to [`Self::create_locked`].
    #[allow(clippy::too_many_arguments)]
    pub fn create_bump_locked(
        store: &'a dyn JournalStore,
        clock: &'a dyn Clock,
        idgen: &dyn IdGen,
        paths: JournalPaths,
        plan_id: String,
        version: String,
        targets: Vec<String>,
        head_sha: String,
        bump: crate::protocol::journal::BumpInputs,
        lock: Box<dyn JournalLock>,
    ) -> io::Result<Self> {
        Self::create_inner_locked(
            store,
            clock,
            idgen,
            paths,
            plan_id,
            version,
            targets,
            Some(head_sha),
            Some(bump),
            lock,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_inner(
        store: &'a dyn JournalStore,
        clock: &'a dyn Clock,
        idgen: &dyn IdGen,
        paths: JournalPaths,
        plan_id: String,
        version: String,
        targets: Vec<String>,
        head_sha: Option<String>,
        bump: Option<crate::protocol::journal::BumpInputs>,
    ) -> io::Result<Self> {
        let lock = store.lock_exclusive(&paths.lock_file())?;
        Self::create_inner_locked(
            store, clock, idgen, paths, plan_id, version, targets, head_sha, bump, lock,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_inner_locked(
        store: &'a dyn JournalStore,
        clock: &'a dyn Clock,
        idgen: &dyn IdGen,
        paths: JournalPaths,
        plan_id: String,
        version: String,
        targets: Vec<String>,
        head_sha: Option<String>,
        bump: Option<crate::protocol::journal::BumpInputs>,
        lock: Box<dyn JournalLock>,
    ) -> io::Result<Self> {
        let run_id = idgen.new_id();
        let mut journal = Self {
            store,
            clock,
            paths,
            run_id: run_id.clone(),
            state: RunState::empty(),
            _lock: lock,
        };
        journal.append(EventKind::RunCreated {
            run_id,
            plan_id,
            version,
            targets,
            head_sha,
            bump,
        })?;
        Ok(journal)
    }

    /// Resume an existing run: take the single-active-cut lock, rebuild state from
    /// the journal (the source of truth), and best-effort re-persist the manifest
    /// cache so it reflects the log even if a prior crash left it stale.
    ///
    /// A manifest-write failure here is **not** fatal: the manifest is disposable
    /// and the in-memory state is already authoritative (rebuilt from the log), so
    /// a transient cache-write error must not brick an otherwise-recoverable run.
    ///
    /// # Errors
    /// [`io::ErrorKind::WouldBlock`] if the lock is held, [`io::ErrorKind::NotFound`]
    /// if the run has no journal, [`io::ErrorKind::InvalidInput`] for a malformed
    /// `run_id`, plus any store/parse error reading the log.
    pub fn open(
        store: &'a dyn JournalStore,
        clock: &'a dyn Clock,
        paths: JournalPaths,
        run_id: &str,
    ) -> io::Result<Self> {
        validate_run_id(run_id)?;
        let lock = store.lock_exclusive(&paths.lock_file())?;
        let events = read_events(store, &paths.journal_file(run_id))?;
        if events.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no release journal for run {run_id}"),
            ));
        }
        let state = reduce(&events);
        let journal = Self {
            store,
            clock,
            paths,
            run_id: run_id.to_string(),
            state,
            _lock: lock,
        };
        // Best-effort self-heal of the disposable manifest from the log.
        let _ = journal.persist_manifest();
        Ok(journal)
    }

    /// The run id.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// The current materialized state.
    #[must_use]
    pub fn state(&self) -> &RunState {
        &self.state
    }

    /// The resolved paths this journal writes to.
    #[must_use]
    pub fn paths(&self) -> &JournalPaths {
        &self.paths
    }

    /// Record a fact with append-then-apply atomicity, returning the updated
    /// state.
    ///
    /// The sequence is strictly: **(1)** serialize and durably fsync the event to
    /// the log (the source of truth), **(2)** apply it to the in-memory
    /// projection, **(3)** best-effort rewrite the disposable manifest cache.
    ///
    /// Step 3 failing does **not** fail the append: once step 1 returns the event
    /// is committed, so reporting `Err` would tempt the caller to retry an
    /// already-committed fact. A stale manifest self-heals on the next append or
    /// [`Journal::open`]. Only a step-1 failure is fatal, and it leaves the state
    /// untouched.
    ///
    /// This is a low-level append of a raw [`EventKind`] (`seq`/`ts`/
    /// `idempotency_key` are assigned here). It does **not** validate release
    /// state-machine ordering — that policy belongs to the coordinator; the
    /// reducer only guarantees replay-idempotency and terminal-state freezing.
    ///
    /// # Errors
    /// A store error from the durable append (step 1), or an event that fails to
    /// serialize.
    pub fn append(&mut self, kind: EventKind) -> io::Result<&RunState> {
        let event = JournalEvent {
            schema_version: JOURNAL_SCHEMA_VERSION,
            seq: self.state.applied_seq + 1,
            ts: self.clock.now_unix(),
            idempotency_key: kind.idempotency_key(),
            kind,
        };
        let line = serde_json::to_string(&event).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("serialize event: {e}"))
        })?;
        // 1. Durable append FIRST (the event is the source of truth).
        self.store
            .append_line(&self.paths.journal_file(&self.run_id), &line)?;
        // 2. Apply to the in-memory projection.
        apply(&mut self.state, &event);
        // 3. Best-effort rewrite of the disposable manifest cache.
        let _ = self.persist_manifest();
        Ok(&self.state)
    }

    /// Serialize the current state and atomically replace the manifest cache.
    fn persist_manifest(&self) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("serialize manifest: {e}"),
            )
        })?;
        self.store
            .write_atomic(&self.paths.manifest_file(&self.run_id), &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::journal::{PublishReceipt, TagState};
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    // ── In-memory fakes for the ports ──────────────────────────────────────

    #[derive(Default)]
    struct StoreInner {
        /// path → full byte contents (journal lines are stored joined by "\n").
        files: HashMap<PathBuf, Vec<u8>>,
        /// currently-held lock paths (single-active-cut simulation).
        locked: HashSet<PathBuf>,
        /// when set, the next `write_atomic` fails (crash-injection).
        fail_next_atomic: bool,
    }

    #[derive(Clone, Default)]
    struct FakeStore {
        inner: Rc<RefCell<StoreInner>>,
    }

    impl FakeStore {
        fn journal_lines(&self, path: &Path) -> Vec<String> {
            self.inner
                .borrow()
                .files
                .get(path)
                .map(|b| {
                    String::from_utf8_lossy(b)
                        .lines()
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        }

        fn arm_atomic_failure(&self) {
            self.inner.borrow_mut().fail_next_atomic = true;
        }
    }

    /// The lock guard: removes its path from `locked` on drop.
    struct FakeLock {
        inner: Rc<RefCell<StoreInner>>,
        path: PathBuf,
    }

    impl JournalLock for FakeLock {}

    impl Drop for FakeLock {
        fn drop(&mut self) {
            self.inner.borrow_mut().locked.remove(&self.path);
        }
    }

    impl JournalStore for FakeStore {
        fn lock_exclusive(&self, lock_path: &Path) -> io::Result<Box<dyn JournalLock>> {
            let mut inner = self.inner.borrow_mut();
            if inner.locked.contains(lock_path) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another release cut holds the lock",
                ));
            }
            inner.locked.insert(lock_path.to_path_buf());
            Ok(Box::new(FakeLock {
                inner: Rc::clone(&self.inner),
                path: lock_path.to_path_buf(),
            }))
        }

        fn append_line(&self, path: &Path, line: &str) -> io::Result<()> {
            let mut inner = self.inner.borrow_mut();
            let buf = inner.files.entry(path.to_path_buf()).or_default();
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
            Ok(())
        }

        fn read_lines(&self, path: &Path) -> io::Result<Vec<String>> {
            Ok(self.journal_lines(path))
        }

        fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
            Ok(self.inner.borrow().files.get(path).cloned())
        }

        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let mut inner = self.inner.borrow_mut();
            if inner.fail_next_atomic {
                inner.fail_next_atomic = false;
                return Err(io::Error::other("injected atomic-write crash"));
            }
            inner.files.insert(path.to_path_buf(), bytes.to_vec());
            Ok(())
        }

        fn list_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
            let inner = self.inner.borrow();
            let mut names: HashSet<String> = HashSet::new();
            for path in inner.files.keys() {
                // Any file under `dir/<name>/…` contributes `<name>`.
                if let Ok(rest) = path.strip_prefix(dir) {
                    if let Some(first) = rest.components().next() {
                        names.insert(first.as_os_str().to_string_lossy().into_owned());
                    }
                }
            }
            Ok(names.into_iter().collect())
        }
    }

    struct FakeClock {
        t: std::cell::Cell<u64>,
    }
    impl FakeClock {
        fn at(t: u64) -> Self {
            Self {
                t: std::cell::Cell::new(t),
            }
        }
    }
    impl Clock for FakeClock {
        fn now_unix(&self) -> u64 {
            let now = self.t.get();
            self.t.set(now + 1); // advance so successive events get distinct ts
            now
        }
    }

    struct FakeIdGen {
        id: String,
    }
    impl IdGen for FakeIdGen {
        fn new_id(&self) -> String {
            self.id.clone()
        }
    }

    struct FakeGit {
        common_dir: PathBuf,
    }
    impl GitRepo for FakeGit {
        fn head_commit(&self) -> io::Result<String> {
            Ok("deadbeef".into())
        }
        fn is_work_tree(&self) -> bool {
            true
        }
        fn shortlog(&self, _since: Option<&str>) -> io::Result<String> {
            Ok(String::new())
        }
        fn tags(&self) -> io::Result<Vec<String>> {
            Ok(Vec::new())
        }
        fn git_common_dir(&self) -> io::Result<PathBuf> {
            Ok(self.common_dir.clone())
        }
    }

    fn paths() -> JournalPaths {
        JournalPaths::new("/repo/.git/ossctl/releases")
    }

    fn receipt(version: &str) -> PublishReceipt {
        PublishReceipt {
            ecosystem: "cargo".into(),
            package: Some("shipshape".into()),
            version: version.into(),
            registry_url: Some("https://crates.io/crates/shipshape".into()),
            digest: Some("sha256:abc".into()),
        }
    }

    /// A representative full run's worth of events, seq 1..=N.
    fn sample_events() -> Vec<JournalEvent> {
        let kinds = vec![
            EventKind::RunCreated {
                run_id: "RUN01".into(),
                plan_id: "plan-abc".into(),
                version: "0.1.0".into(),
                targets: vec!["cargo".into(), "npm".into()],
                head_sha: None,
                bump: None,
            },
            EventKind::PhaseEntered {
                phase: Phase::DryRun,
            },
            EventKind::TargetDryRun {
                target: "cargo".into(),
            },
            EventKind::PhaseCompleted {
                phase: Phase::DryRun,
                outcome: PhaseOutcome::Ok,
            },
            EventKind::PhaseEntered {
                phase: Phase::Publish,
            },
            EventKind::TargetPublished {
                target: "cargo".into(),
                receipt: receipt("0.1.0"),
            },
        ];
        kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| JournalEvent {
                schema_version: JOURNAL_SCHEMA_VERSION,
                seq: (i + 1) as u64,
                ts: 1000 + i as u64,
                idempotency_key: kind.idempotency_key(),
                kind,
            })
            .collect()
    }

    // ── Path resolution ────────────────────────────────────────────────────

    #[test]
    fn paths_resolve_under_git_common_dir() {
        let git = FakeGit {
            common_dir: PathBuf::from("/repo/.git"),
        };
        let p = JournalPaths::from_git(&git, None).unwrap();
        assert_eq!(p.releases_dir(), Path::new("/repo/.git/ossctl/releases"));
        assert_eq!(
            p.journal_file("RUN01"),
            Path::new("/repo/.git/ossctl/releases/RUN01/journal.jsonl")
        );
        assert_eq!(
            p.manifest_file("RUN01"),
            Path::new("/repo/.git/ossctl/releases/RUN01/manifest.json")
        );
        assert_eq!(p.lock_file(), Path::new("/repo/.git/ossctl/releases/.lock"));
    }

    #[test]
    fn path_override_wins_over_git() {
        let git = FakeGit {
            common_dir: PathBuf::from("/repo/.git"),
        };
        let p = JournalPaths::from_git(&git, Some(Path::new("/ci/journal"))).unwrap();
        assert_eq!(p.releases_dir(), Path::new("/ci/journal"));
    }

    // ── Reducer: determinism + idempotency ─────────────────────────────────

    #[test]
    fn reduce_is_deterministic() {
        let events = sample_events();
        let a = reduce(&events);
        let b = reduce(&events);
        assert_eq!(a, b);
        assert_eq!(a.run_id, "RUN01");
        assert_eq!(a.plan_id, "plan-abc");
        assert_eq!(a.targets, vec!["cargo".to_string(), "npm".to_string()]);
        assert!(a.dry_run.contains("cargo"));
        assert_eq!(a.published.get("cargo").unwrap().version, "0.1.0");
        // DryRun completed → current phase cleared; Publish entered.
        assert_eq!(a.current_phase, Some(Phase::Publish));
        assert_eq!(a.applied_seq, 6);
    }

    #[test]
    fn reduce_ignores_slice_order() {
        let mut events = sample_events();
        events.reverse();
        let out = reduce(&events);
        // Same result as in-order despite the reversed slice.
        assert_eq!(out, reduce(&sample_events()));
    }

    #[test]
    fn replaying_a_seen_event_is_a_no_op() {
        let events = sample_events();
        let mut state = reduce(&events);
        let before = state.clone();
        // Re-apply an already-folded event (seq below the watermark): no change.
        apply(&mut state, &events[2]);
        assert_eq!(state, before);
        // Re-apply the whole stream on top: still no change (watermark holds).
        for ev in &events {
            apply(&mut state, ev);
        }
        assert_eq!(state, before);
    }

    #[test]
    fn structural_idempotency_of_publish_and_tags() {
        // Two publishes of the same target with distinct seq → one map entry, and
        // the later receipt wins deterministically.
        let mut state = RunState::empty();
        let mk = |seq: u64, kind: EventKind| JournalEvent {
            schema_version: JOURNAL_SCHEMA_VERSION,
            seq,
            ts: seq,
            idempotency_key: kind.idempotency_key(),
            kind,
        };
        apply(
            &mut state,
            &mk(
                1,
                EventKind::RunCreated {
                    run_id: "R".into(),
                    plan_id: "p".into(),
                    version: "0.1.0".into(),
                    targets: vec!["cargo".into()],
                    head_sha: None,
                    bump: None,
                },
            ),
        );
        apply(
            &mut state,
            &mk(
                2,
                EventKind::TargetPublished {
                    target: "cargo".into(),
                    receipt: receipt("0.1.0"),
                },
            ),
        );
        apply(
            &mut state,
            &mk(
                3,
                EventKind::TargetPublished {
                    target: "cargo".into(),
                    receipt: receipt("0.1.1"),
                },
            ),
        );
        assert_eq!(state.published.len(), 1);
        assert_eq!(state.published.get("cargo").unwrap().version, "0.1.1");

        apply(
            &mut state,
            &mk(
                4,
                EventKind::TagCreatedLocal {
                    tag: "v0.1.1".into(),
                },
            ),
        );
        apply(
            &mut state,
            &mk(
                5,
                EventKind::TagPushedRemote {
                    tag: "v0.1.1".into(),
                },
            ),
        );
        assert_eq!(
            state.tags.get("v0.1.1"),
            Some(&TagState {
                created_local: true,
                pushed_remote: true,
                github_release: false,
                github_release_url: None,
                github_release_delegated: false,
            })
        );
    }

    #[test]
    fn github_release_delegation_reduces_to_the_tag_state_flag() {
        // A CI-delegated cut records `github_release_delegated` in place of
        // `github_release_created`: the reducer sets the delegation flag and leaves
        // `github_release` false (coordinator-release-vs-cargo-dist-ownership).
        let mut state = RunState::empty();
        let mk = |seq: u64, kind: EventKind| JournalEvent {
            schema_version: JOURNAL_SCHEMA_VERSION,
            seq,
            ts: seq,
            idempotency_key: kind.idempotency_key(),
            kind,
        };
        apply(
            &mut state,
            &mk(
                1,
                EventKind::TagCreatedLocal {
                    tag: "v1.0.0".into(),
                },
            ),
        );
        apply(
            &mut state,
            &mk(
                2,
                EventKind::TagPushedRemote {
                    tag: "v1.0.0".into(),
                },
            ),
        );
        apply(
            &mut state,
            &mk(
                3,
                EventKind::GithubReleaseDelegated {
                    tag: "v1.0.0".into(),
                    delegated_to: "cargo-dist".into(),
                },
            ),
        );
        assert_eq!(
            state.tags.get("v1.0.0"),
            Some(&TagState {
                created_local: true,
                pushed_remote: true,
                github_release: false,
                github_release_url: None,
                github_release_delegated: true,
            })
        );
    }

    #[test]
    fn verify_phase_ok_completes_a_v5_run_even_when_a_v6_binary_resumes_it() {
        let mut events = sample_events();
        // Run semantics are fixed by run_created, not by the newer binary that
        // appends a later completion event during resume.
        events[0].schema_version = 5;
        let mut state = reduce(&events);
        assert_eq!(state.status, RunStatus::InProgress);
        let mut seq = state.applied_seq;
        let mut push = |state: &mut RunState, phase: Phase| {
            seq += 1;
            apply(
                state,
                &JournalEvent {
                    schema_version: JOURNAL_SCHEMA_VERSION,
                    seq,
                    ts: 9000 + seq,
                    idempotency_key: format!("phase_completed:{}", phase.as_str()),
                    kind: EventKind::PhaseCompleted {
                        phase,
                        outcome: PhaseOutcome::Ok,
                    },
                },
            );
        };
        // A v2 Tag `Ok` no longer completes the run — the post-tag Dist barrier does.
        push(&mut state, Phase::Tag);
        assert_eq!(state.status, RunStatus::InProgress);
        push(&mut state, Phase::Dist);
        assert_eq!(state.status, RunStatus::InProgress);
        push(&mut state, Phase::Verify);
        assert_eq!(state.status, RunStatus::Completed);
    }

    #[test]
    fn a_v4_dist_ok_stays_completed_for_backward_compat() {
        let mut events = sample_events();
        events[0].schema_version = 4;
        let mut state = reduce(&events);
        let seq = state.applied_seq + 1;
        apply(
            &mut state,
            &JournalEvent {
                schema_version: 4,
                seq,
                ts: 9000,
                idempotency_key: "phase_completed:dist".into(),
                kind: EventKind::PhaseCompleted {
                    phase: Phase::Dist,
                    outcome: PhaseOutcome::Ok,
                },
            },
        );
        assert_eq!(state.status, RunStatus::Completed);
    }

    #[test]
    fn a_v1_tag_ok_completes_the_run_for_backward_compat() {
        // A v1 journal (schema_version 1, no Dist phase) ended at `Tag Ok`. The
        // reducer must still read it as Completed, or an upgraded binary would
        // misreport a finished release as InProgress (and disagree with the manifest
        // cache the old reducer wrote as Completed).
        let mut events = sample_events();
        events[0].schema_version = 1;
        let mut state = reduce(&events);
        assert_eq!(state.status, RunStatus::InProgress);
        let seq = state.applied_seq + 1;
        apply(
            &mut state,
            &JournalEvent {
                schema_version: 1,
                seq,
                ts: 9000,
                idempotency_key: "phase_completed:tag".into(),
                kind: EventKind::PhaseCompleted {
                    phase: Phase::Tag,
                    outcome: PhaseOutcome::Ok,
                },
            },
        );
        assert_eq!(state.status, RunStatus::Completed);
    }

    #[test]
    fn run_abandoned_is_terminal_with_reason() {
        let mut state = reduce(&sample_events());
        let seq = state.applied_seq + 1;
        apply(
            &mut state,
            &JournalEvent {
                schema_version: JOURNAL_SCHEMA_VERSION,
                seq,
                ts: 9000,
                idempotency_key: "run_abandoned".into(),
                kind: EventKind::RunAbandoned {
                    reason: "OTP timeout".into(),
                },
            },
        );
        assert_eq!(state.status, RunStatus::Abandoned);
        assert_eq!(state.abandon_reason.as_deref(), Some("OTP timeout"));
    }

    // ── Journal handle: create / append / manifest round-trip ──────────────

    #[test]
    fn create_writes_run_created_and_manifest() {
        let store = FakeStore::default();
        let clock = FakeClock::at(1000);
        let idgen = FakeIdGen { id: "RUN01".into() };
        let journal = Journal::create(
            &store,
            &clock,
            &idgen,
            paths(),
            "plan-abc".into(),
            "0.1.0".into(),
            vec!["cargo".into()],
        )
        .unwrap();
        assert_eq!(journal.run_id(), "RUN01");
        assert_eq!(journal.state().run_id, "RUN01");
        assert_eq!(journal.state().applied_seq, 1);

        // One durable event line was written…
        let lines = store.journal_lines(&paths().journal_file("RUN01"));
        assert_eq!(lines.len(), 1);
        // …and the manifest cache deserializes back to the same state.
        let manifest = store
            .inner
            .borrow()
            .files
            .get(&paths().manifest_file("RUN01"))
            .cloned()
            .unwrap();
        let loaded: RunState = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(&loaded, journal.state());
    }

    #[test]
    fn append_records_facts_and_a_failed_phase_can_later_complete_ok() {
        // Regression: an earlier design deduped appends by idempotency key, which
        // silently swallowed a Failed→Ok phase retry after a resume. The log now
        // records both facts and the reducer's upsert lands the final outcome.
        let store = FakeStore::default();
        let clock = FakeClock::at(1000);
        let idgen = FakeIdGen { id: "RUN01".into() };
        let mut journal = Journal::create(
            &store,
            &clock,
            &idgen,
            paths(),
            "plan-abc".into(),
            "0.1.0".into(),
            vec!["cargo".into()],
        )
        .unwrap();
        journal
            .append(EventKind::PhaseCompleted {
                phase: Phase::Publish,
                outcome: PhaseOutcome::Failed,
            })
            .unwrap();
        journal
            .append(EventKind::PhaseCompleted {
                phase: Phase::Publish,
                outcome: PhaseOutcome::Ok,
            })
            .unwrap();
        // Both facts are durable (RunCreated + two PhaseCompleted).
        let lines = store.journal_lines(&paths().journal_file("RUN01"));
        assert_eq!(lines.len(), 3);
        // The projection reflects the final Ok outcome, once.
        let publish = journal
            .state()
            .phases
            .iter()
            .filter(|r| r.phase == Phase::Publish)
            .collect::<Vec<_>>();
        assert_eq!(publish.len(), 1);
        assert_eq!(publish[0].outcome, PhaseOutcome::Ok);
    }

    #[test]
    fn terminal_state_freezes_further_events() {
        // After abandonment, a stray later event must not mutate the projection.
        let mut state = reduce(&sample_events());
        let published_before = state.published.clone();
        let mut seq = state.applied_seq;
        let mut next = |kind: EventKind| {
            seq += 1;
            JournalEvent {
                schema_version: JOURNAL_SCHEMA_VERSION,
                seq,
                ts: 9000 + seq,
                idempotency_key: kind.idempotency_key(),
                kind,
            }
        };
        apply(
            &mut state,
            &next(EventKind::RunAbandoned {
                reason: "aborted".into(),
            }),
        );
        assert_eq!(state.status, RunStatus::Abandoned);
        // A publish recorded after abandonment is ignored.
        apply(
            &mut state,
            &next(EventKind::TargetPublished {
                target: "npm".into(),
                receipt: receipt("9.9.9"),
            }),
        );
        assert_eq!(state.status, RunStatus::Abandoned);
        assert_eq!(state.published, published_before);
        assert!(!state.published.contains_key("npm"));
    }

    // ── Append-then-apply crash safety ─────────────────────────────────────

    #[test]
    fn event_survives_a_manifest_write_crash() {
        let store = FakeStore::default();
        let clock = FakeClock::at(1000);
        let idgen = FakeIdGen { id: "RUN01".into() };
        let mut journal = Journal::create(
            &store,
            &clock,
            &idgen,
            paths(),
            "plan-abc".into(),
            "0.1.0".into(),
            vec!["cargo".into()],
        )
        .unwrap();
        // Arm a crash on the NEXT manifest write, then append: the durable event
        // lands first and the append still SUCCEEDS (the manifest is disposable),
        // leaving only a stale cache.
        store.arm_atomic_failure();
        journal
            .append(EventKind::TargetPublished {
                target: "cargo".into(),
                receipt: receipt("0.1.0"),
            })
            .unwrap();
        drop(journal); // release the lock

        // Reopen: state is rebuilt from the authoritative journal, so the event's
        // effect is present despite the manifest write having crashed.
        let clock2 = FakeClock::at(2000);
        let reopened = Journal::open(&store, &clock2, paths(), "RUN01").unwrap();
        assert!(reopened.state().published.contains_key("cargo"));
        assert_eq!(reopened.state().applied_seq, 2);
        // The self-heal on open rewrote the manifest to match the log.
        let manifest = store
            .inner
            .borrow()
            .files
            .get(&paths().manifest_file("RUN01"))
            .cloned()
            .unwrap();
        let loaded: RunState = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(&loaded, reopened.state());
    }

    #[test]
    fn open_reduces_from_journal_when_manifest_absent() {
        // Simulate a wiped manifest but intact journal: write only event lines.
        let store = FakeStore::default();
        for ev in sample_events() {
            let line = serde_json::to_string(&ev).unwrap();
            store
                .append_line(&paths().journal_file("RUN01"), &line)
                .unwrap();
        }
        let clock = FakeClock::at(1000);
        let journal = Journal::open(&store, &clock, paths(), "RUN01").unwrap();
        assert_eq!(journal.state(), &reduce(&sample_events()));
    }

    // ── flock mutual exclusion ─────────────────────────────────────────────

    #[test]
    fn second_create_fails_while_lock_is_held() {
        let store = FakeStore::default();
        let clock = FakeClock::at(1000);
        let idgen = FakeIdGen { id: "RUN01".into() };
        let held = Journal::create(
            &store,
            &clock,
            &idgen,
            paths(),
            "plan-abc".into(),
            "0.1.0".into(),
            vec!["cargo".into()],
        )
        .unwrap();

        // A concurrent cut must fail fast while the first holds the lock.
        let clock2 = FakeClock::at(2000);
        let idgen2 = FakeIdGen { id: "RUN02".into() };
        // `Journal` is not `Debug` (it holds trait-object ports), so inspect the
        // error without `unwrap_err`.
        let result = Journal::create(
            &store,
            &clock2,
            &idgen2,
            paths(),
            "plan-def".into(),
            "0.1.0".into(),
            vec!["cargo".into()],
        );
        let err = result.err().expect("concurrent create must fail");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);

        // Once the first handle drops, the lock frees and a new cut succeeds.
        drop(held);
        let clock3 = FakeClock::at(3000);
        let idgen3 = FakeIdGen { id: "RUN02".into() };
        assert!(Journal::create(
            &store,
            &clock3,
            &idgen3,
            paths(),
            "plan-def".into(),
            "0.1.0".into(),
            vec!["cargo".into()],
        )
        .is_ok());
    }

    // ── read-only helpers ──────────────────────────────────────────────────

    #[test]
    fn load_state_is_read_only_and_takes_no_lock() {
        let store = FakeStore::default();
        let clock = FakeClock::at(1000);
        let idgen = FakeIdGen { id: "RUN01".into() };
        let journal = Journal::create(
            &store,
            &clock,
            &idgen,
            paths(),
            "plan-abc".into(),
            "0.1.0".into(),
            vec!["cargo".into()],
        )
        .unwrap();
        // The lock is still held by `journal`; load_state must not need it.
        let loaded = load_state(&store, &paths(), "RUN01").unwrap().unwrap();
        assert_eq!(&loaded, journal.state());
        assert!(load_state(&store, &paths(), "MISSING").unwrap().is_none());
    }

    #[test]
    fn load_state_prefers_manifest_but_falls_back_to_journal() {
        let store = FakeStore::default();
        let clock = FakeClock::at(1000);
        let idgen = FakeIdGen { id: "RUN01".into() };
        let journal = Journal::create(
            &store,
            &clock,
            &idgen,
            paths(),
            "plan-abc".into(),
            "0.1.0".into(),
            vec!["cargo".into()],
        )
        .unwrap();
        let expected = journal.state().clone();
        drop(journal);

        // Fast path: manifest present → returned as-is.
        assert_eq!(
            load_state(&store, &paths(), "RUN01").unwrap(),
            Some(expected.clone())
        );

        // Corrupt the manifest → falls back to reducing the authoritative journal.
        store
            .write_atomic(&paths().manifest_file("RUN01"), b"{not json")
            .unwrap();
        assert_eq!(
            load_state(&store, &paths(), "RUN01").unwrap(),
            Some(expected)
        );
    }

    #[test]
    fn read_run_state_reduces_from_journal_without_writing() {
        // Write only event lines (no manifest), simulating the authoritative log.
        let store = FakeStore::default();
        for ev in sample_events() {
            let line = serde_json::to_string(&ev).unwrap();
            store
                .append_line(&paths().journal_file("RUN01"), &line)
                .unwrap();
        }
        // Snapshot every file before the read so we can prove nothing was written.
        let files_before = store.inner.borrow().files.clone();

        let state = read_run_state(&store, &paths(), "RUN01").unwrap().unwrap();
        assert_eq!(state, reduce(&sample_events()));

        // Read-only: no manifest self-heal, no lock file, no new bytes anywhere.
        assert_eq!(
            store.inner.borrow().files,
            files_before,
            "read_run_state must not write anything"
        );
        assert!(
            store.inner.borrow().locked.is_empty(),
            "read_run_state must not take the lock"
        );
        assert!(
            !store
                .inner
                .borrow()
                .files
                .contains_key(&paths().manifest_file("RUN01")),
            "read_run_state must not materialize a manifest"
        );

        // A run with no journal is a clean None, and a bad id is rejected.
        assert!(read_run_state(&store, &paths(), "MISSING")
            .unwrap()
            .is_none());
        assert_eq!(
            read_run_state(&store, &paths(), "../escape")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn read_run_returns_events_and_reduced_state() {
        // `release show` reads the log and its projection together, read-only.
        let store = FakeStore::default();
        for ev in sample_events() {
            let line = serde_json::to_string(&ev).unwrap();
            store
                .append_line(&paths().journal_file("RUN01"), &line)
                .unwrap();
        }
        let files_before = store.inner.borrow().files.clone();

        let (events, state) = read_run(&store, &paths(), "RUN01").unwrap().unwrap();
        assert_eq!(events, sample_events());
        assert_eq!(state, reduce(&sample_events()));

        // Read-only: no manifest self-heal, no lock, no new bytes.
        assert_eq!(store.inner.borrow().files, files_before);
        assert!(store.inner.borrow().locked.is_empty());

        // A missing run is a clean None; a traversal id is rejected.
        assert!(read_run(&store, &paths(), "MISSING").unwrap().is_none());
        assert_eq!(
            read_run(&store, &paths(), "../escape").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn read_run_state_ignores_a_stale_manifest_fast_path() {
        // A manifest that lags the log must NOT shadow the authoritative reduce —
        // this is exactly the crash window `verify` must see through.
        let store = FakeStore::default();
        for ev in sample_events() {
            let line = serde_json::to_string(&ev).unwrap();
            store
                .append_line(&paths().journal_file("RUN01"), &line)
                .unwrap();
        }
        // Plant a deliberately-wrong manifest cache.
        let mut stale = RunState::empty();
        stale.run_id = "RUN01".into();
        stale.plan_id = "STALE".into();
        store
            .write_atomic(
                &paths().manifest_file("RUN01"),
                &serde_json::to_vec(&stale).unwrap(),
            )
            .unwrap();

        // load_state trusts the (stale) manifest; read_run_state reduces the log.
        assert_eq!(
            load_state(&store, &paths(), "RUN01")
                .unwrap()
                .unwrap()
                .plan_id,
            "STALE"
        );
        assert_eq!(
            read_run_state(&store, &paths(), "RUN01")
                .unwrap()
                .unwrap()
                .plan_id,
            "plan-abc",
        );
    }

    #[test]
    fn run_id_validation_rejects_path_traversal() {
        let store = FakeStore::default();
        let clock = FakeClock::at(1000);
        for bad in ["..", "a/b", "", ".", "x/../y"] {
            assert_eq!(
                load_state(&store, &paths(), bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "load_state must reject run id {bad:?}"
            );
            let err = Journal::open(&store, &clock, paths(), bad).err().unwrap();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn list_runs_excludes_lock_and_stray_files() {
        let store = FakeStore::default();
        let clock = FakeClock::at(1000);
        for id in ["RUN01", "RUN02"] {
            let idgen = FakeIdGen { id: id.into() };
            let _j = Journal::create(
                &store,
                &clock,
                &idgen,
                paths(),
                "plan".into(),
                "0.1.0".into(),
                vec!["cargo".into()],
            )
            .unwrap();
        }
        // A leftover atomic-write temp file and a stray top-level file must not be
        // reported as runs (only entries with a non-empty journal count).
        store
            .write_atomic(&paths().releases_dir().join("journal.jsonl.tmp"), b"x")
            .unwrap();
        store
            .write_atomic(&paths().releases_dir().join(".lock"), b"")
            .unwrap();
        let runs = list_runs(&store, &paths()).unwrap();
        assert_eq!(runs, vec!["RUN01".to_string(), "RUN02".to_string()]);
    }

    // ── forward tolerance ──────────────────────────────────────────────────

    #[test]
    fn read_events_refuses_a_too_new_schema_version() {
        let store = FakeStore::default();
        let mut ev = sample_events()[0].clone();
        ev.schema_version = JOURNAL_SCHEMA_VERSION + 1;
        let line = serde_json::to_string(&ev).unwrap();
        store
            .append_line(&paths().journal_file("RUN01"), &line)
            .unwrap();
        let err = read_events(&store, &paths().journal_file("RUN01")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_events_tolerates_unknown_additive_fields() {
        // An extra top-level field a newer shipshape might add is ignored, not fatal.
        let store = FakeStore::default();
        let line = r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"R","plan_id":"p","version":"0.1.0","targets":[],"future_field":42}"#;
        store
            .append_line(&paths().journal_file("RUN01"), line)
            .unwrap();
        let events = read_events(&store, &paths().journal_file("RUN01")).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, 1);
    }

    #[test]
    fn run_status_as_str_matches_serde() {
        for s in [
            RunStatus::InProgress,
            RunStatus::Completed,
            RunStatus::Abandoned,
        ] {
            assert_eq!(
                serde_json::to_value(s).unwrap(),
                serde_json::Value::String(s.as_str().to_string()),
                "as_str() drifted from serde for {s:?}"
            );
        }
    }

    #[test]
    fn read_events_skips_blank_lines() {
        let store = FakeStore::default();
        store
            .append_line(&paths().journal_file("RUN01"), "")
            .unwrap();
        assert!(read_events(&store, &paths().journal_file("RUN01"))
            .unwrap()
            .is_empty());
    }
}
