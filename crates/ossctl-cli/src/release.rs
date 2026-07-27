//! `ossctl release …` handlers.
//!
//! `release plan` is implemented (the sealed content-addressed approval seam,
//! ADR-0002 §3); the remaining verbs (`cut`/`resume`/`verify`/`show`/`list`/
//! `abandon`) live in `ossctl-core::release` and land with their sibling units,
//! returning a clean `not_implemented` envelope until then. The argument shapes
//! are real so the surface and `--help` are accurate.

use std::io::Write;
use std::path::PathBuf;

use clap::Args;

use ossctl_core::contract::schema::Status;
use ossctl_core::contract::{self, LoadError, Normalized};
use ossctl_core::ports::GitRepo;
use ossctl_core::protocol::journal::{
    EventKind, JournalEvent, RunState, RunStatus, JOURNAL_SCHEMA_VERSION,
};
use ossctl_core::protocol::plan::ReleasePlan;
use ossctl_core::protocol::reconcile::ReconcileReport;
use ossctl_core::release::adapters::EffectCtx;
use ossctl_core::release::coordinator::{self, CutError, ProgressSink};
use ossctl_core::release::journal::{self, Journal, JournalPaths};

use crate::cli::ReleaseAction;
use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::{
    ReadOnlyJournalStore, RealClock, RealCommandRunner, RealFs, RealGitRepo, RealIdGen,
    RealJournalStore, RealRegistryQuery, RealTagger,
};

/// Arguments for `release plan`.
#[derive(Args, Debug)]
pub struct PlanArgs {
    /// Repository root to plan a release for (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// The chosen release version to seal into the plan (the human's bump per
    /// design §3.4). The binary never prompts and never derives it here — the
    /// skill supplies the approved version as validated input.
    #[arg(long, value_name = "VERSION")]
    pub version: String,
}

/// Arguments for `release cut`.
///
/// `release plan` is read-only and persists nothing, so `cut` re-derives the plan
/// from the *current* repo state and the re-supplied `--version`, then refuses
/// unless the recomputed `plan_id` equals the approved `--plan` (drift check,
/// ADR-0002 §3). The `--version` cannot be recovered from the opaque `plan_id`
/// hash, so it is required; a wrong version simply fails the drift check.
#[derive(Args, Debug)]
pub struct CutArgs {
    /// The sealed plan id to execute (from `release plan`). The cut refuses if the
    /// current repository no longer hashes to it.
    #[arg(long, value_name = "PLAN_ID")]
    pub plan: String,
    /// The chosen release version the plan was sealed with (the human's approved
    /// bump). Must match the version `release plan` sealed, or the cut refuses on
    /// drift.
    #[arg(long, value_name = "VERSION")]
    pub version: String,
    /// Repository root to cut the release in (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Override the release-journal location (default: the repo's
    /// `git-common-dir/ossctl/releases`). For CI or debugging (ADR-0003 §3).
    #[arg(long, value_name = "DIR")]
    pub journal_dir: Option<PathBuf>,
}

/// A single positional `<run_id>` plus the journal-location flags, shared by
/// `resume` / `verify` / `show` — every run-scoped verb needs to locate the same
/// journal for the same run.
#[derive(Args, Debug)]
pub struct RunIdArgs {
    /// The run id.
    #[arg(value_name = "RUN_ID")]
    pub run_id: String,
    /// Repository whose release journal to read (default: current directory). The
    /// journal is rooted at `<git-common-dir>/ossctl/releases`, so any linked
    /// worktree of the repo resolves to the same run state.
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Read the journal from this directory instead of resolving it from git
    /// (`<git-common-dir>/ossctl/releases`). For CI and tests.
    #[arg(long, value_name = "PATH")]
    pub journal_dir: Option<PathBuf>,
}

/// Arguments for `release resume` — the shared run-scoped locators plus the
/// explicit go-ahead for the state table's unverifiable (`Unknown`) rows.
#[derive(Args, Debug)]
pub struct ResumeArgs {
    /// The run id to resume.
    #[arg(value_name = "RUN_ID")]
    pub run_id: String,
    /// Repository whose release journal to resume (default: current directory).
    /// The journal is rooted at `<git-common-dir>/ossctl/releases`, so any linked
    /// worktree of the repo resolves to the same run state.
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Read/write the journal in this directory instead of resolving it from git
    /// (`<git-common-dir>/ossctl/releases`). For CI and tests.
    #[arg(long, value_name = "PATH")]
    pub journal_dir: Option<PathBuf>,
    /// Proceed past targets whose remote state could not be verified (registry
    /// outage, an unobservable distribution target). Without this the resume
    /// refuses on any `unknown` target rather than assume it did not publish
    /// (ADR-0003 §4). With it, an unverifiable target is trusted to the journal:
    /// a recorded publish is skipped, an unrecorded one is (re-)published.
    #[arg(long)]
    pub allow_unverified: bool,
}

/// Arguments for `release abandon`.
#[derive(Args, Debug)]
pub struct AbandonArgs {
    /// The run id to abandon.
    #[arg(value_name = "RUN_ID")]
    pub run_id: String,
    /// Why the run is being abandoned (journaled).
    #[arg(long, value_name = "TEXT")]
    pub reason: String,
}

/// Dispatch a `release` subcommand to its handler.
pub fn dispatch(action: ReleaseAction, format: OutputFormat) -> Result<(), CliError> {
    match action {
        ReleaseAction::Plan(args) => plan(&args, format),
        ReleaseAction::Cut(args) => cut(&args, format),
        ReleaseAction::Resume(args) => resume(&args, format),
        ReleaseAction::Verify(args) => verify(&args, format),
        ReleaseAction::Show(args) => show(&args, format),
        ReleaseAction::List => Err(CliError::not_implemented("release list")),
        ReleaseAction::Abandon(_) => Err(CliError::not_implemented("release abandon")),
    }
}

/// `ossctl release plan` — compute and seal a content-addressed release plan
/// (read-only; mutates no external state).
///
/// Obtains the contract through the same normalizer behind `contract show` and
/// the facts through the same detector behind `facts`, reads git `HEAD`, then
/// seals the plan in `ossctl-core::release::plan` — so `plan`, the audit, and
/// `/oss-init` agree on the contract down to the byte (ADR-0001 §3). Emits the
/// plan and its `plan_id` under the canonical envelope.
pub fn plan(args: &PlanArgs, format: OutputFormat) -> Result<(), CliError> {
    let version = validate_version(&args.version)?;

    let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
    if !repo_root.is_dir() {
        return Err(CliError::user(
            "invalid_repo_root",
            format!("repo_root '{}' is not a directory", repo_root.display()),
        )
        .with_invalid_value(repo_root.display().to_string()));
    }
    // Canonicalize so the plan's inputs are absolute + symlink-resolved, matching
    // the facts detector's contract.
    let root = std::fs::canonicalize(&repo_root).map_err(|e| {
        CliError::system(
            "io_error",
            format!(
                "cannot canonicalize repo_root '{}': {e}",
                repo_root.display()
            ),
        )
    })?;

    // A plan is sealed against a normalized contract; a missing or invalid
    // OSS-RELEASE.md is the same failure `contract show` reports.
    let normalized = contract::normalize(&root, &RealFs).map_err(load_error_to_cli)?;
    if !normalized.is_valid() {
        return Err(invalid_contract_error(&normalized));
    }

    let git = RealGitRepo::new(&root);
    // A release must be sealed against a concrete commit. An unborn repo is the
    // caller-fixable case (commit first); any other git failure (git missing,
    // corrupt repo, permissions) is preserved in the message rather than
    // mislabelled "no commits".
    let head_sha = git.head_commit().map_err(|e| {
        CliError::user(
            "no_head",
            format!("cannot plan a release: could not resolve HEAD ({e}) — the repository may have no commits"),
        )
    })?;

    let facts = ossctl_core::facts::gather(&root, &RealFs, &git);

    let plan = ossctl_core::release::plan::build(&normalized.contract, &facts, &head_sha, version);

    let mut warnings = normalized.problems.warnings.clone();
    // Surface a non-blocking warning when the contract configures nothing to
    // publish — the plan would tag only.
    if plan.targets.is_empty() {
        warnings.push(
            "the contract declares no publish targets — this plan would create the git tag only"
                .to_string(),
        );
    }
    // A target whose package is still null after facts-resolution is ambiguous
    // (a monorepo with several crates of one ecosystem): the executor will infer
    // it at cut time. Name it so the approver knows the plan is not fully
    // concrete and can pin an explicit `package` in the contract.
    for t in plan.targets.iter().filter(|t| t.package.is_none()) {
        warnings.push(format!(
            "target '{}' has no resolved package name (ambiguous or undetected) — this plan is \
             NOT cuttable as-is; `release cut` will refuse it. Pin an explicit 'package' in the \
             contract and re-plan",
            t.ecosystem.as_str()
        ));
    }

    match format {
        OutputFormat::Json => crate::output::emit_json(&plan, &warnings)?,
        OutputFormat::Text => render_plan_text(&plan, &warnings),
    }
    Ok(())
}

/// `ossctl release verify` — read-only reconcile of a journaled run against
/// remote registry state (ADR-0002 §1, ADR-0003 state table).
///
/// Reads the run's state straight from the authoritative event log (no lock, no
/// manifest self-heal, no writes) and, for each published target, dispatches the
/// ecosystem adapter's `verify()` against the registry through the injected
/// [`RealRegistryQuery`] port — classifying each as `matches`/`conflicts`/
/// `missing`/`unknown`. A registry lookup that cannot be performed degrades to
/// `unknown`, never a false `missing`. Emits the reconcile report under the
/// canonical envelope. This command mutates nothing — not the repo, the journal,
/// or the registry.
pub fn verify(args: &RunIdArgs, format: OutputFormat) -> Result<(), CliError> {
    let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
    if !repo_root.is_dir() {
        return Err(CliError::user(
            "invalid_repo_root",
            format!("repo_root '{}' is not a directory", repo_root.display()),
        )
        .with_invalid_value(repo_root.display().to_string()));
    }
    let root = std::fs::canonicalize(&repo_root).map_err(|e| {
        CliError::system(
            "io_error",
            format!(
                "cannot canonicalize repo_root '{}': {e}",
                repo_root.display()
            ),
        )
    })?;

    // Resolve the journal root: an explicit `--journal-dir` wins (CI/tests),
    // otherwise `<git-common-dir>/ossctl/releases` so every linked worktree shares
    // one run-state root.
    let git = RealGitRepo::new(&root);
    let paths = JournalPaths::from_git(&git, args.journal_dir.as_deref()).map_err(|e| {
        CliError::system(
            "journal_root_unresolved",
            format!(
                "cannot locate the release journal root (is '{}' a git repository? pass \
                 --journal-dir to override): {e}",
                root.display()
            ),
        )
    })?;

    // Authoritative, write-free read of the run's state.
    let store = ReadOnlyJournalStore;
    let state = journal::read_run_state(&store, &paths, &args.run_id)
        .map_err(|e| read_state_error(&args.run_id, e))?
        .ok_or_else(|| {
            CliError::user(
                "run_not_found",
                format!(
                    "no release run '{}' found under {}",
                    args.run_id,
                    paths.releases_dir().display()
                ),
            )
            .with_invalid_value(args.run_id.clone())
        })?;

    // The reconcile queries the registry only; the runner/clock are supplied
    // because the adapter effect context requires them, but verify() never runs a
    // command or reads the clock.
    let runner = RealCommandRunner;
    let clock = RealClock;
    let registry = RealRegistryQuery;
    let ctx = EffectCtx {
        runner: &runner,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
    };
    let report = ossctl_core::release::reconcile::reconcile(&state, &ctx);
    let warnings = reconcile_warnings(&state, &report);

    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &warnings)?,
        OutputFormat::Text => render_reconcile_text(&report, &warnings),
    }
    Ok(())
}

/// Map a journal-read `io::Error` to the right exit class: a malformed `run_id`
/// is caller-fixable (exit 1); a corrupt or too-new journal is a system fault
/// (exit 2).
fn read_state_error(run_id: &str, e: std::io::Error) -> CliError {
    match e.kind() {
        std::io::ErrorKind::InvalidInput => {
            CliError::user("invalid_run_id", e.to_string()).with_invalid_value(run_id.to_string())
        }
        std::io::ErrorKind::InvalidData => CliError::system("journal_unreadable", e.to_string()),
        _ => CliError::system("io_error", e.to_string()),
    }
}

/// Non-fatal context for the envelope: targets declared but never published (the
/// run was interrupted before publishing them — nothing to reconcile, and *not* a
/// false `missing`), and a note when the run is still live.
fn reconcile_warnings(
    state: &ossctl_core::protocol::journal::RunState,
    report: &ReconcileReport,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if state.status == RunStatus::InProgress {
        warnings.push(
            "the run is still in progress — this reconcile is a point-in-time snapshot".to_string(),
        );
    }
    for target in &state.targets {
        if state.published.contains_key(target) {
            continue;
        }
        // A cancelled target has a known reason on the journal — report that,
        // rather than the generic "not yet published" that would misread an
        // intentional skip as an interruption.
        if let Some(reason) = state.cancelled.get(target) {
            warnings.push(format!("target '{target}' was cancelled: {reason}"));
        } else {
            warnings.push(format!(
                "target '{target}' was declared but has no publish receipt in this run \
                 (not yet published, or the run was interrupted); it is not reconciled"
            ));
        }
    }
    if report.summary.conflicts > 0 {
        warnings.push(format!(
            "{} target(s) conflict with registry state — a human must reconcile before resuming",
            report.summary.conflicts
        ));
    }
    warnings
}

fn render_reconcile_text(report: &ReconcileReport, warnings: &[String]) {
    println!("run_id:     {}", report.run_id);
    println!("plan_id:    {}", report.plan_id);
    println!(
        "status:     {} (journal seq {})",
        report.run_status.as_str(),
        report.journal_seq
    );
    let s = &report.summary;
    println!(
        "reconciled: {} ({} matches, {} conflicts, {} missing, {} unknown)",
        s.reconciled, s.matches, s.conflicts, s.missing, s.unknown
    );
    for t in &report.targets {
        println!(
            "  {:<10} {:<8} {:<20} {}",
            t.target,
            t.ecosystem,
            format!("{}@{}", t.package.as_deref().unwrap_or("<none>"), t.version),
            t.outcome.as_str(),
        );
        if let Some(detail) = &t.detail {
            println!("             └─ {detail}");
        }
    }
    for w in warnings {
        println!("warning:    {w}");
    }
}

/// The most-recent journal events `release show` returns as the §12 "recent
/// event window". A release journal is inherently small (a handful of events per
/// target and phase), but a resumed, many-times-retried run can grow, so the
/// window is bounded rather than dumping the full log on every poll.
const SHOW_EVENT_WINDOW: usize = 100;

/// The `data` body of a `release show` progress query — the §12 progress-query
/// payload: the folded run state, the last journal sequence folded into it, and a
/// bounded window of recent events. Stable across live and terminal runs so an
/// agent parses one shape regardless of when it polls.
#[derive(serde::Serialize)]
struct ShowSnapshot<'a> {
    /// The last journal sequence folded into [`Self::state`] — the poll cursor an
    /// agent advances on. Surfaced under a stable public name rather than
    /// `RunState`'s internal `applied_seq` watermark.
    last_seq: u64,
    /// The folded run state: identity, derived status, per-target/tag progress.
    state: &'a RunState,
    /// The tail of the journal (at most [`SHOW_EVENT_WINDOW`] events), in
    /// ascending `seq` — the "recent event window" a live poller reads
    /// incrementally.
    recent_events: &'a [JournalEvent],
}

/// `ossctl release show <run_id>` — the §12 progress query for a release run:
/// poll a run's live progress, or read its post-mortem summary.
///
/// Reads the run's event log and reduced state read-only (no lock, no manifest
/// self-heal, no writes — safe against a live cut) and, in `--json` mode, **always**
/// emits the canonical `{schema_version, data, warnings}` envelope whose `data` is
/// a [`ShowSnapshot`]: the folded [`RunState`], the last folded `seq`, and a
/// bounded [`SHOW_EVENT_WINDOW`] window of recent events. The shape is **the same
/// whether the run is live or terminal** — §12 forbids a progress query silently
/// switching wire format across polls, so an agent parses one document regardless
/// of when it catches the run. (A live JSONL *stream* is `release cut`'s job; a
/// poll returns a framed snapshot, exactly the payload §12 defines: current state,
/// last seq, and a recent event window.) Text mode renders a human summary.
pub fn show(args: &RunIdArgs, format: OutputFormat) -> Result<(), CliError> {
    // `show` is a pure journal read: when an explicit `--journal-dir` is given it
    // needs neither a repository nor a valid cwd (a post-mortem query against an
    // archived journal must work from anywhere). Only the git-resolved default
    // requires a repo root.
    let paths = if let Some(dir) = args.journal_dir.as_deref() {
        JournalPaths::new(dir)
    } else {
        let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
        if !repo_root.is_dir() {
            return Err(CliError::user(
                "invalid_repo_root",
                format!("repo_root '{}' is not a directory", repo_root.display()),
            )
            .with_invalid_value(repo_root.display().to_string()));
        }
        let root = std::fs::canonicalize(&repo_root).map_err(|e| {
            CliError::system(
                "io_error",
                format!(
                    "cannot canonicalize repo_root '{}': {e}",
                    repo_root.display()
                ),
            )
        })?;
        let git = RealGitRepo::new(&root);
        JournalPaths::from_git(&git, None).map_err(|e| {
            CliError::system(
                "journal_root_unresolved",
                format!(
                    "cannot locate the release journal root (is '{}' a git repository? pass \
                     --journal-dir to override): {e}",
                    root.display()
                ),
            )
        })?
    };

    // Read-only read of both the event log and its projection.
    let store = ReadOnlyJournalStore;
    let (events, state) = journal::read_run(&store, &paths, &args.run_id)
        .map_err(|e| read_state_error(&args.run_id, e))?
        .ok_or_else(|| {
            CliError::user(
                "run_not_found",
                format!(
                    "no release run '{}' found under {}",
                    args.run_id,
                    paths.releases_dir().display()
                ),
            )
            .with_invalid_value(args.run_id.clone())
        })?;

    // The bounded recent-event window (the log tail, ascending seq).
    let window = &events[events.len().saturating_sub(SHOW_EVENT_WINDOW)..];

    match format {
        OutputFormat::Json => {
            let snapshot = ShowSnapshot {
                last_seq: state.applied_seq,
                state: &state,
                recent_events: window,
            };
            crate::output::emit_json(&snapshot, &show_warnings(&state))?;
        }
        OutputFormat::Text => render_show_text(&state, window),
    }
    Ok(())
}

/// Non-fatal context for the progress-query envelope: the abandon reason (always),
/// and — for a *terminal* run — any declared target that never got a publish
/// receipt (interrupted, or cancelled with its reason), so a finished run's gaps
/// are visible without re-reading the event window. A live run's unpublished
/// targets are simply not-done-yet, not gaps, so they are not warned about.
fn show_warnings(state: &RunState) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(reason) = &state.abandon_reason {
        warnings.push(format!("run was abandoned: {reason}"));
    }
    let terminal = matches!(state.status, RunStatus::Completed | RunStatus::Abandoned);
    if !terminal {
        return warnings;
    }
    for target in &state.targets {
        if state.published.contains_key(target) {
            continue;
        }
        if let Some(reason) = state.cancelled.get(target) {
            warnings.push(format!("target '{target}' was cancelled: {reason}"));
        } else {
            warnings.push(format!(
                "target '{target}' was declared but has no publish receipt in this run"
            ));
        }
    }
    warnings
}

/// Render a run's state as a human progress summary (text mode) — identity,
/// status, phase progress, per-target landing state, tags, then the event window.
/// Works for a live or a terminal run; the status line says which.
fn render_show_text(state: &RunState, events: &[JournalEvent]) {
    println!("run_id:     {}", state.run_id);
    println!("plan_id:    {}", state.plan_id);
    println!("version:    {}", state.version);
    match state.status {
        RunStatus::Abandoned => match &state.abandon_reason {
            Some(reason) => println!(
                "status:     abandoned ({reason}) (journal seq {})",
                state.applied_seq
            ),
            None => println!("status:     abandoned (journal seq {})", state.applied_seq),
        },
        status => {
            let phase = state
                .current_phase
                .map(|p| format!(" — in {}", p.as_str()))
                .unwrap_or_default();
            println!(
                "status:     {}{phase} (journal seq {})",
                status.as_str(),
                state.applied_seq
            );
        }
    }

    println!("targets:    {}", state.targets.len());
    for target in &state.targets {
        let landing = if let Some(receipt) = state.published.get(target) {
            format!("published @{}", receipt.version)
        } else if let Some(reason) = state.cancelled.get(target) {
            format!("cancelled ({reason})")
        } else if state.built.contains(target) {
            "built".to_string()
        } else if state.dry_run.contains(target) {
            "dry-run ok".to_string()
        } else {
            "pending".to_string()
        };
        println!("  {target:<10} {landing}");
    }

    for (tag, tstate) in &state.tags {
        let mut steps = Vec::new();
        if tstate.created_local {
            steps.push("local");
        }
        if tstate.pushed_remote {
            steps.push("pushed");
        }
        if tstate.github_release {
            steps.push("release");
        }
        println!("tag {tag}: {}", steps.join(", "));
    }

    println!("events:     {}", events.len());
    for event in events {
        println!("  {}", render_event_line(event));
    }
}

/// `ossctl release resume <run_id>` — reconcile an interrupted run against remote
/// registry state (remote is ground truth) and continue the phase barrier from the
/// first incomplete step (ADR-0003 §4).
///
/// Flow: open the run's journal under the single-active-cut lock (authoritative
/// reduce of the durable log); short-circuit a terminal run (a `completed` run is
/// idempotent success, an `abandoned` run is refused). Otherwise re-derive the
/// approved plan from the *current* repo + the journal's sealed version and refuse
/// unless it still hashes to the run's `plan_id` (the same drift discipline `cut`
/// enforces — a resume must continue the exact approved plan, whose tag points at
/// the sealed commit). Then **reconcile**: classify every target against remote via
/// the adapter `verify()` per the ADR-0003 state table. A hard-stop cell
/// (`conflicts`/`missing` after a recorded publish, or an unverifiable target
/// without `--allow-unverified`) refuses with the §10 envelope and mutates nothing.
/// A publish that landed without a durable receipt is **adopted forward** (journalled)
/// so it is never re-published, then the coordinator continues — already-landed
/// targets skipped, tag-once preserved.
///
/// Output (§12): the same event stream as `cut` — with `--json`, one
/// [`JournalEvent`] per line; otherwise human progress lines.
pub fn resume(args: &ResumeArgs, format: OutputFormat) -> Result<(), CliError> {
    let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
    if !repo_root.is_dir() {
        return Err(CliError::user(
            "invalid_repo_root",
            format!("repo_root '{}' is not a directory", repo_root.display()),
        )
        .with_invalid_value(repo_root.display().to_string()));
    }
    let root = std::fs::canonicalize(&repo_root).map_err(|e| {
        CliError::system(
            "io_error",
            format!(
                "cannot canonicalize repo_root '{}': {e}",
                repo_root.display()
            ),
        )
    })?;

    let git = RealGitRepo::new(&root);
    let paths = JournalPaths::from_git(&git, args.journal_dir.as_deref()).map_err(|e| {
        CliError::system(
            "journal_root_unresolved",
            format!(
                "cannot locate the release journal root (is '{}' a git repository? pass \
                 --journal-dir to override): {e}",
                root.display()
            ),
        )
    })?;

    let store = RealJournalStore;
    let clock = RealClock;
    let runner = RealCommandRunner;
    let registry = RealRegistryQuery;
    let tagger = RealTagger::new(&root);

    // Open the run under the single-active-cut lock: an authoritative reduce of the
    // durable log (never the possibly-stale manifest fast-path), held for the whole
    // resume so no concurrent cut/resume can race this reconcile.
    let mut journal = Journal::open(&store, &clock, paths, &args.run_id)
        .map_err(|e| open_run_error(&args.run_id, e))?;

    // Terminal runs never resume: a completed run is idempotent success (nothing to
    // do), an abandoned run is refused (it was deliberately marked un-resumable).
    match journal.state().status {
        RunStatus::Completed => {
            if !matches!(format, OutputFormat::Json) {
                println!(
                    "run {} is already complete — nothing to resume",
                    args.run_id
                );
            }
            return Ok(());
        }
        RunStatus::Abandoned => {
            return Err(CliError::user(
                "run_abandoned",
                format!(
                    "run {} was abandoned{} — it cannot be resumed; plan and cut a new release",
                    args.run_id,
                    journal
                        .state()
                        .abandon_reason
                        .as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default(),
                ),
            )
            .with_invalid_value(args.run_id.clone()));
        }
        RunStatus::InProgress => {}
    }

    // Re-derive the approved plan from CURRENT repo state + the journal's sealed
    // version, refusing on drift (the same discipline `cut` enforces).
    let plan = derive_resume_plan(&root, &git, journal.state(), &args.run_id)?;

    let ctx = EffectCtx {
        runner: &runner,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
    };

    // Reconcile against remote registry state (the state table). This is the only
    // step that decides continue/skip/adopt/stop; the coordinator then executes it.
    let reconcile = ossctl_core::release::resume::reconcile_for_resume(
        journal.state(),
        &plan,
        &ctx,
        args.allow_unverified,
    );
    if reconcile.is_blocked() {
        return Err(resume_conflict_error(&args.run_id, &reconcile));
    }

    let mut sink = StreamSink::new(std::io::stdout(), matches!(format, OutputFormat::Json));
    // Lead the stream with the run's identity (parity with `cut`), so a `--json`
    // resume is self-contained even when the only following events are adoptions.
    stream_run_created(&mut sink, &journal, &plan);

    // Adopt forward any publish that landed without a durable receipt, so the
    // coordinator treats it as done and never re-publishes an already-published
    // version. Journalled before the barrier continues, and streamed like any fact.
    journal_adoptions(&mut journal, &reconcile, &mut sink, &args.run_id)?;

    // Continue the phase-barrier from the first incomplete step. The coordinator's
    // idempotent re-entry skips completed phases and already-landed targets, and
    // drives tag-once (idempotent step-by-step) — no second copy of that logic here.
    match coordinator::execute(&mut journal, &plan, &ctx, &tagger, &mut sink) {
        Ok(()) => {
            if !matches!(format, OutputFormat::Json) {
                render_cut_success(&args.run_id, &plan);
            }
            Ok(())
        }
        Err(e) => Err(cut_error_to_cli(&args.run_id, e)),
    }
}

/// Re-derive the run's approved plan from the *current* repo + the journal's sealed
/// version and refuse unless it still hashes to the run's `plan_id` — a resume must
/// continue the exact approved plan (whose tag points at the sealed commit), so a
/// drifted repo is a hard stop, not a silently-different release.
///
/// Runs the same normalizer/detector/approval/executability gates `cut` does; the
/// only new gate is `resume_drift` (the current `plan_id` ≠ the run's).
fn derive_resume_plan(
    root: &std::path::Path,
    git: &RealGitRepo,
    state: &ossctl_core::protocol::journal::RunState,
    run_id: &str,
) -> Result<ReleasePlan, CliError> {
    let normalized = contract::normalize(root, &RealFs).map_err(load_error_to_cli)?;
    if !normalized.is_valid() {
        return Err(invalid_contract_error(&normalized));
    }
    // A resume mutates external state (it may publish + tag), so — like `cut` — it
    // refuses a contract a human has not approved.
    if normalized.contract.status != Status::Approved {
        return Err(CliError::user(
            "not_approved",
            format!(
                "{} is `{}`, not `approved` — a human must approve the contract before resuming",
                contract::CONTRACT_FILENAME,
                normalized.contract.status.as_str()
            ),
        )
        .with_invalid_value(normalized.contract.status.as_str().to_string()));
    }
    let head_sha = git.head_commit().map_err(|e| {
        CliError::user(
            "no_head",
            format!("cannot resume a release: could not resolve HEAD ({e}) — the repository may have no commits"),
        )
    })?;
    let facts = ossctl_core::facts::gather(root, &RealFs, git);
    let plan =
        ossctl_core::release::plan::build(&normalized.contract, &facts, &head_sha, &state.version);
    if plan.plan_id != state.plan_id {
        return Err(resume_drift_error(state, &plan));
    }
    // Defense in depth: the same executability preflight `cut` runs.
    coordinator::validate_plan(&plan).map_err(|e| cut_error_to_cli(run_id, e))?;
    Ok(plan)
}

/// Journal each adopt-forward publish (a publish that landed without a durable
/// receipt) as a `target_published` fact **before** the barrier continues, so the
/// coordinator skips it rather than re-publishing an already-published version. Each
/// fact is streamed to `sink` the same way the coordinator streams its own.
fn journal_adoptions(
    journal: &mut Journal<'_>,
    reconcile: &ossctl_core::release::resume::ResumeReconcile,
    sink: &mut dyn ProgressSink,
    run_id: &str,
) -> Result<(), CliError> {
    for (target, receipt) in reconcile.adoptions() {
        let kind = EventKind::TargetPublished {
            target: target.to_string(),
            receipt: receipt.clone(),
        };
        let idempotency_key = kind.idempotency_key();
        let kind_for_sink = kind.clone();
        let state = journal.append(kind).map_err(|e| {
            CliError::system(
                "journal_error",
                format!("run {run_id}: could not journal an adopted publish receipt: {e}"),
            )
        })?;
        sink.event(&JournalEvent {
            schema_version: JOURNAL_SCHEMA_VERSION,
            seq: state.applied_seq,
            ts: state.updated_ts,
            idempotency_key,
            kind: kind_for_sink,
        });
    }
    Ok(())
}

/// Map a `Journal::open` failure (resume) to the right §10 envelope: an absent run
/// or bad id is caller-fixable (exit 1); a held lock is the single-active-cut
/// refusal (exit 1 — wait/abandon); a corrupt/too-new journal is a system fault
/// (exit 2).
fn open_run_error(run_id: &str, e: std::io::Error) -> CliError {
    match e.kind() {
        std::io::ErrorKind::NotFound => CliError::user(
            "run_not_found",
            format!("no release run '{run_id}' found to resume"),
        )
        .with_invalid_value(run_id.to_string()),
        std::io::ErrorKind::WouldBlock => CliError::user(
            "cut_in_progress",
            "another release cut or resume is already active for this repository (the \
             single-active-cut lock is held) — wait for it, or `release abandon` a stuck run"
                .to_string(),
        ),
        std::io::ErrorKind::InvalidInput => {
            CliError::user("invalid_run_id", e.to_string()).with_invalid_value(run_id.to_string())
        }
        std::io::ErrorKind::InvalidData => CliError::system("journal_unreadable", e.to_string()),
        _ => CliError::system("journal_error", e.to_string()),
    }
}

/// The §10 `resume_drift` refusal: the current repo no longer hashes to the run's
/// approved `plan_id`, so a commit/contract/version change occurred since the cut.
/// Exit 1 — the run must be resumed against the state it was sealed for (check out
/// the sealed commit), or a new release planned. `ossctl` will not continue a
/// *different* plan under the old run.
fn resume_drift_error(
    state: &ossctl_core::protocol::journal::RunState,
    current: &ReleasePlan,
) -> CliError {
    CliError::user(
        "resume_drift",
        format!(
            "run {} was sealed against plan {}, but the current repository (HEAD {}, version {}) \
             hashes to a different plan_id — a commit, a contract or manifest edit, a version \
             change, or an uncommitted working-tree change occurred since the cut (the plan is \
             re-derived from the working tree, so a dirty tree drifts too). Restore the sealed \
             state (a clean checkout of the sealed commit), or plan and cut a new release; ossctl \
             will not continue a different plan under this run",
            state.run_id,
            short_sha(&state.plan_id),
            short_sha(&current.head_sha),
            current.version,
        ),
    )
    .with_invalid_value(state.run_id.clone())
    .with_expected(serde_json::json!({
        "sealed_plan_id": state.plan_id,
        "current_plan_id": current.plan_id,
    }))
}

/// The §10 `resume_conflict` refusal: one or more targets are in a state the resume
/// must not continue past — a recorded publish that now conflicts or has vanished,
/// or an unverifiable target with no `--allow-unverified` go-ahead (ADR-0003 §4).
/// Exit 1 — a human must reconcile the registries (or pass the go-ahead) before
/// resuming; `ossctl` never overwrites or blind-re-publishes.
fn resume_conflict_error(
    run_id: &str,
    reconcile: &ossctl_core::release::resume::ResumeReconcile,
) -> CliError {
    let blockers = reconcile.blockers();
    let problems: Vec<String> = blockers
        .iter()
        .map(|d| {
            format!(
                "{} ({}): {} — {}",
                d.target,
                d.outcome.as_str(),
                d.action.as_str(),
                d.detail
                    .as_deref()
                    .unwrap_or("must be reconciled by a human"),
            )
        })
        .collect();
    CliError::user(
        "resume_conflict",
        format!(
            "run {run_id} cannot be resumed: {} target(s) are in a state a resume must not \
             continue past (remote is ground truth). Reconcile the registries — or pass \
             --allow-unverified for targets that only could not be verified — then resume again",
            blockers.len()
        ),
    )
    .with_invalid_value(run_id.to_string())
    .with_problems(problems)
}
/// `ossctl release cut --plan <id> --version <v>` — execute a sealed plan across
/// the phase-barrier coordinator, refusing on repo drift (ADR-0002 §2/§3).
///
/// Flow: re-derive the plan from the *current* contract + facts + `HEAD` and the
/// supplied `--version`; **refuse (`plan_stale`) unless the recomputed `plan_id`
/// equals `--plan`** (the drift check — a commit, contract edit, or version
/// change since `release plan` aborts here rather than publishing something the
/// human did not approve). On acceptance, create the journalled run (single
/// active cut) and drive dry-run-all → build-all → publish-all → tag-once through
/// the coordinator, streaming each journalled fact.
///
/// Output (§12): a `--output=jsonl`-style event stream — with `--json`, one
/// [`JournalEvent`] per line; otherwise human progress lines. `cut` never emits a
/// single `--json` envelope (a partially-irreversible, streaming command).
///
/// On any phase failure the run **stops with no rollback**; what landed is
/// journalled and the error names the `run_id` for `release verify` / `resume`.
pub fn cut(args: &CutArgs, format: OutputFormat) -> Result<(), CliError> {
    let version = validate_version(&args.version)?;

    let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
    if !repo_root.is_dir() {
        return Err(CliError::user(
            "invalid_repo_root",
            format!("repo_root '{}' is not a directory", repo_root.display()),
        )
        .with_invalid_value(repo_root.display().to_string()));
    }
    let root = std::fs::canonicalize(&repo_root).map_err(|e| {
        CliError::system(
            "io_error",
            format!(
                "cannot canonicalize repo_root '{}': {e}",
                repo_root.display()
            ),
        )
    })?;

    // Re-derive the same normalized contract + facts + HEAD the plan sealed
    // against, through the identical code paths behind `contract show` / `facts`.
    let normalized = contract::normalize(&root, &RealFs).map_err(load_error_to_cli)?;
    if !normalized.is_valid() {
        return Err(invalid_contract_error(&normalized));
    }
    // A cut mutates external state, so — unlike the read-only `plan` — it refuses a
    // contract a human has not approved (SCHEMA.md: mutating members require
    // `status: approved`).
    if normalized.contract.status != Status::Approved {
        return Err(CliError::user(
            "not_approved",
            format!(
                "{} is `{}`, not `approved` — a human must approve the contract before a cut",
                contract::CONTRACT_FILENAME,
                normalized.contract.status.as_str()
            ),
        )
        .with_invalid_value(normalized.contract.status.as_str().to_string()));
    }

    let git = RealGitRepo::new(&root);
    let head_sha = git.head_commit().map_err(|e| {
        CliError::user(
            "no_head",
            format!("cannot cut a release: could not resolve HEAD ({e}) — the repository may have no commits"),
        )
    })?;
    let facts = ossctl_core::facts::gather(&root, &RealFs, &git);

    // Drift check: the current repo + supplied version must hash to the approved
    // plan_id, or we refuse rather than publish a different release (§3).
    let current =
        ossctl_core::release::plan::build(&normalized.contract, &facts, &head_sha, version);
    if current.plan_id != args.plan {
        return Err(plan_stale_error(&args.plan, &current));
    }

    // Preflight the plan *before* creating a run, so an unexecutable plan (an
    // unresolved package, a duplicate-ecosystem target) is refused up front rather
    // than leaving an orphaned `run_created` run behind.
    coordinator::validate_plan(&current).map_err(|e| cut_error_to_cli("(not created)", e))?;

    // Journal location (git-common-dir-local, or an explicit override).
    let paths = JournalPaths::from_git(&git, args.journal_dir.as_deref()).map_err(|e| {
        CliError::system(
            "io_error",
            format!("cannot resolve the release-journal directory: {e}"),
        )
    })?;

    let store = RealJournalStore;
    let clock = RealClock;
    let idgen = RealIdGen;
    let runner = RealCommandRunner;
    let registry = RealRegistryQuery;
    let tagger = RealTagger::new(&root);

    // Create the run under the single-active-cut lock (RunCreated is journalled).
    let target_ids: Vec<String> = current
        .targets
        .iter()
        .map(|t| t.ecosystem.as_str().to_string())
        .collect();
    let mut journal = Journal::create(
        &store,
        &clock,
        &idgen,
        paths,
        current.plan_id.clone(),
        current.version.clone(),
        target_ids,
    )
    .map_err(create_journal_error)?;
    let run_id = journal.run_id().to_string();

    let mut sink = StreamSink::new(std::io::stdout(), matches!(format, OutputFormat::Json));
    // Emit the run identity first so the stream is self-contained.
    stream_run_created(&mut sink, &journal, &current);

    let ctx = EffectCtx {
        runner: &runner,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
    };

    match coordinator::execute(&mut journal, &current, &ctx, &tagger, &mut sink) {
        Ok(()) => {
            if !matches!(format, OutputFormat::Json) {
                render_cut_success(&run_id, &current);
            }
            Ok(())
        }
        Err(e) => Err(cut_error_to_cli(&run_id, e)),
    }
}

/// The §10 `plan_stale` refusal: the current repo no longer hashes to the
/// approved plan (ADR-0002 §3). Exit 1 — caller-fixable by re-planning.
fn plan_stale_error(approved: &str, current: &ReleasePlan) -> CliError {
    CliError::user(
        "plan_stale",
        format!(
            "the approved plan is stale: the current repository (HEAD {}, version {}) hashes to \
             a different plan_id, so a commit, contract edit, or version change occurred since \
             `release plan` — re-run `ossctl release plan` and approve the new plan_id before cutting",
            short_sha(&current.head_sha),
            current.version,
        ),
    )
    .with_invalid_value(approved.to_string())
    .with_expected(serde_json::json!({ "current_plan_id": current.plan_id }))
}

/// Map a `Journal::create` failure to the error envelope: a held lock is the
/// single-active-cut refusal (user-fixable — wait/abandon), anything else is a
/// journal I/O failure (system).
fn create_journal_error(e: std::io::Error) -> CliError {
    if e.kind() == std::io::ErrorKind::WouldBlock {
        CliError::user(
            "cut_in_progress",
            "another release cut or resume is already active for this repository (the \
             single-active-cut lock is held) — wait for it, or `release abandon` a stuck run"
                .to_string(),
        )
    } else {
        CliError::system(
            "journal_error",
            format!("could not create the release journal: {e}"),
        )
    }
}

/// Map a coordinator [`CutError`] to the error envelope, always naming the
/// `run_id` and the (no-rollback) recovery path.
fn cut_error_to_cli(run_id: &str, err: CutError) -> CliError {
    match err {
        CutError::Plan(message) => {
            // Caught before any external action — a plan the executor cannot run.
            CliError::user("invalid_plan", message)
        }
        CutError::Journal(io) => CliError::system(
            "journal_error",
            format!("run {run_id}: could not write the release journal: {io} — the run may be in an unknown state"),
        ),
        CutError::PhaseFailed { .. } => CliError::system(
            "release_failed",
            format!(
                "run {run_id}: {err}. Nothing was rolled back; the journal records exactly what \
                 landed under this run id. Recovery via `release verify {run_id}` / `release \
                 resume {run_id}` lands in a later version; until then inspect the journal and \
                 reconcile the registries manually before retrying"
            ),
        ),
    }
}

/// A [`ProgressSink`] that streams each journalled fact as a JSONL line
/// (`--json`, §12) or a human-readable progress line, shared by `release cut`'s
/// live stream and `release show`'s live tail (both surface the same journal
/// events in the same shape).
///
/// Writes through an injected [`Write`] (production: a `stdout` handle) and
/// flushes each line, so a JSONL consumer (`tail -f`, `jq`) sees events as they
/// happen rather than at buffer flush. A **broken pipe** (the consumer exited,
/// e.g. piped to `head`) is not an error to shout about: `stopped` latches and
/// the sink goes quiet for the rest of the run rather than letting the write
/// panic mid-stream — the journal remains the durable record either way. The
/// writer is a type parameter so the broken-pipe latch is unit-testable against
/// a failing writer without a real pipe.
struct StreamSink<W: Write> {
    out: W,
    json: bool,
    stopped: bool,
}

impl<W: Write> StreamSink<W> {
    fn new(out: W, json: bool) -> Self {
        Self {
            out,
            json,
            stopped: false,
        }
    }
}

impl<W: Write> ProgressSink for StreamSink<W> {
    fn event(&mut self, event: &JournalEvent) {
        if self.stopped {
            return;
        }
        // A `JournalEvent` (plain strings/enums/ints) is infallible to serialize;
        // a failure here is a programmer error, not a runtime condition to hide.
        let line = if self.json {
            serde_json::to_string(event).expect("a JournalEvent is always serializable")
        } else {
            render_event_line(event)
        };
        if writeln!(self.out, "{line}")
            .and_then(|()| self.out.flush())
            .is_err()
        {
            // Consumer went away (broken pipe) — stop streaming quietly; the
            // journal remains the durable record.
            self.stopped = true;
        }
    }
}

/// Emit the `run_created` fact to the stream before the coordinator runs, so the
/// event stream carries the run's identity as its first line.
fn stream_run_created(sink: &mut dyn ProgressSink, journal: &Journal<'_>, plan: &ReleasePlan) {
    let state = journal.state();
    let event = JournalEvent {
        schema_version: JOURNAL_SCHEMA_VERSION,
        seq: 1,
        ts: state.created_ts,
        idempotency_key: "run_created".to_string(),
        kind: EventKind::RunCreated {
            run_id: journal.run_id().to_string(),
            plan_id: plan.plan_id.clone(),
            version: plan.version.clone(),
            targets: state.targets.clone(),
        },
    };
    sink.event(&event);
}

/// Render one journalled event as a human progress line (text mode).
fn render_event_line(event: &JournalEvent) -> String {
    use ossctl_core::protocol::journal::PhaseOutcome;
    match &event.kind {
        EventKind::RunCreated {
            run_id, targets, ..
        } => {
            format!("run {run_id} started ({} target(s))", targets.len())
        }
        EventKind::PhaseEntered { phase } => format!("→ {}", phase.as_str()),
        EventKind::PhaseCompleted { phase, outcome } => match outcome {
            PhaseOutcome::Ok => format!("✓ {} complete", phase.as_str()),
            PhaseOutcome::Failed => format!("✗ {} failed", phase.as_str()),
        },
        EventKind::TargetDryRun { target } => format!("  dry-run ok: {target}"),
        EventKind::TargetBuilt { target } => format!("  built: {target}"),
        EventKind::TargetPublished { target, receipt } => {
            format!("  published: {target}@{}", receipt.version)
        }
        EventKind::TargetCancelled { target, reason } => {
            format!("  cancelled: {target} ({reason})")
        }
        EventKind::TagCreatedLocal { tag } => format!("  tag created: {tag}"),
        EventKind::TagPushedRemote { tag } => format!("  tag pushed: {tag}"),
        EventKind::GithubReleaseCreated { tag, url } => match url {
            Some(u) => format!("  release: {tag} ({u})"),
            None => format!("  release: {tag}"),
        },
        EventKind::RunAbandoned { reason } => format!("run abandoned: {reason}"),
    }
}

/// Text summary printed after a successful cut (json mode's summary is the stream).
fn render_cut_success(run_id: &str, plan: &ReleasePlan) {
    println!();
    println!("release complete — run {run_id}");
    println!("version: {}", plan.version);
    println!("tag:     v{}", plan.version);
    println!("published {} target(s)", plan.targets.len());
}

/// Short (first 12 hex chars) `HEAD` sha for the drift message.
fn short_sha(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// Validate the chosen version as an opaque, already-chosen identifier. Scheme
/// specificity (semver vs a calver pattern) belongs to the contract/skill, so
/// this only rejects the shapes that could never be a safe version and would be
/// footguns downstream (empty, whitespace, control characters, or a leading `-`
/// that a later git/registry command would read as a flag).
fn validate_version(version: &str) -> Result<&str, CliError> {
    let reject = |msg: &str| {
        Err(CliError::user("invalid_version", msg.to_string())
            .with_invalid_value(version.to_string()))
    };
    if version.is_empty() {
        return reject("the release version must not be empty");
    }
    if version.chars().any(char::is_whitespace) {
        return reject("the release version must not contain whitespace");
    }
    if version.chars().any(char::is_control) {
        return reject("the release version must not contain control characters");
    }
    if version.starts_with('-') {
        return reject("the release version must not start with '-' (it would be read as a flag)");
    }
    // The version becomes the `v{version}` git tag (coordinator tag phase). Reject
    // the shapes `git check-ref-format` forbids, so an invalid ref cannot pass
    // validation, get every package published, and only then fail at tag time
    // (post-publish, unrecoverable-late). This is shape-level ref safety, not
    // scheme validation (semver vs calver stays the contract's job).
    if version.chars().any(|c| "~^:?*[\\".contains(c)) {
        return reject(
            "the release version must not contain any of ~ ^ : ? * [ \\ (invalid in a git tag)",
        );
    }
    if version.contains("..") || version.contains("@{") || version.contains("//") {
        return reject(
            "the release version must not contain '..', '@{', or '//' (invalid in a git tag)",
        );
    }
    // Git rejects a ref component ending in the literal (case-sensitive) `.lock`;
    // this is a ref-name rule, not a filesystem extension check.
    let ends_with_lock = version.as_bytes().ends_with(b".lock");
    if version.starts_with('.') || version.ends_with('.') || ends_with_lock {
        return reject("the release version must not start or end with '.' or end with '.lock' (invalid in a git tag)");
    }
    if version.starts_with('/') || version.ends_with('/') {
        return reject("the release version must not start or end with '/' (invalid in a git tag)");
    }
    Ok(version)
}

fn resolve_repo_root(flag: Option<&PathBuf>) -> Result<PathBuf, CliError> {
    match flag {
        Some(p) => Ok(p.clone()),
        None => std::env::current_dir()
            .map_err(|e| CliError::system("io_error", format!("cannot resolve cwd: {e}"))),
    }
}

/// A failed contract load is a system-level (exit-2) error — the plan could not
/// obtain the config it seals.
fn load_error_to_cli(e: LoadError) -> CliError {
    let code = match e {
        LoadError::NotFound(_) => "contract_not_found",
        LoadError::Io(..) => "io_error",
        LoadError::Utf8(_) => "invalid_encoding",
    };
    CliError::system(code, e.to_string())
}

/// An invalid contract is a caller-fixable (exit-1) error carrying every problem
/// — the plan cannot seal against a config that would not normalize.
fn invalid_contract_error(normalized: &Normalized) -> CliError {
    let problems = &normalized.problems.errors;
    let message = format!(
        "{} would not normalize: {} problem(s) — fix the contract before planning",
        contract::CONTRACT_FILENAME,
        problems.len()
    );
    CliError::user("invalid_contract", message).with_problems(problems.clone())
}

fn render_plan_text(plan: &ReleasePlan, warnings: &[String]) {
    println!("plan_id:    {}", plan.plan_id);
    println!("head:       {}", plan.head_sha);
    println!("version:    {}", plan.version);
    println!("targets:    {}", plan.targets.len());
    for t in &plan.targets {
        println!(
            "  {:<8} {:<12} {:<20} (package: {})",
            t.ecosystem.as_str(),
            t.registry.as_str(),
            t.adapter.as_str(),
            t.package.as_deref().unwrap_or("<inferred at cut>"),
        );
    }
    let phases = plan
        .phases
        .iter()
        .map(|p| p.as_str())
        .collect::<Vec<_>>()
        .join(" → ");
    println!("phases:     {phases}");
    for w in warnings {
        println!("warning:    {w}");
    }
    println!();
    println!("To execute this exact plan (refuses if the repo drifts):");
    println!(
        "  ossctl release cut --plan {} --version {}",
        plan.plan_id, plan.version
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ossctl_core::protocol::journal::EventKind;

    /// A `Write` that always fails, counting attempts — models a reader that
    /// closed the pipe (`| head`).
    #[derive(Default)]
    struct BrokenWriter {
        writes: usize,
    }

    impl Write for BrokenWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "reader went away",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "reader went away",
            ))
        }
    }

    fn run_created(seq: u64) -> JournalEvent {
        let kind = EventKind::RunCreated {
            run_id: "RUN01".to_string(),
            plan_id: "plan-abc".to_string(),
            version: "1.0.0".to_string(),
            targets: vec!["cargo".to_string()],
        };
        JournalEvent {
            schema_version: JOURNAL_SCHEMA_VERSION,
            seq,
            ts: 1000 + seq,
            idempotency_key: kind.idempotency_key(),
            kind,
        }
    }

    /// A broken pipe latches `stopped` on the first failed write, and every later
    /// event is a silent no-op — the write is never retried (no panic, no second
    /// attempt). This is the cut/show stream's broken-pipe safety.
    #[test]
    fn stream_sink_latches_stopped_on_broken_pipe() {
        let mut sink = StreamSink::new(BrokenWriter::default(), true);
        sink.event(&run_created(1));
        assert!(sink.stopped, "a broken pipe must latch stopped");
        assert_eq!(sink.out.writes, 1, "the first event attempts one write");

        // A second event after the latch must not touch the writer at all.
        sink.event(&run_created(2));
        assert_eq!(
            sink.out.writes, 1,
            "a stopped sink must not retry writes on later events"
        );
    }

    /// The happy path emits one compact JSON object per line (JSONL, §12): each
    /// line parses independently and carries the event's `seq`/`kind`.
    #[test]
    fn stream_sink_emits_one_json_object_per_line() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut sink = StreamSink::new(&mut buf, true);
            sink.event(&run_created(1));
            sink.event(&run_created(2));
            assert!(!sink.stopped);
        }
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "one line per event");
        for (i, line) in lines.iter().enumerate() {
            let v: serde_json::Value = serde_json::from_str(line).expect("each line is JSON");
            assert_eq!(v["seq"], (i + 1) as u64);
            assert_eq!(v["kind"], "run_created");
            assert_eq!(v["schema_version"], JOURNAL_SCHEMA_VERSION);
        }
    }

    /// Text mode renders human progress lines, not JSON.
    #[test]
    fn stream_sink_text_mode_renders_human_lines() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut sink = StreamSink::new(&mut buf, false);
            sink.event(&run_created(1));
        }
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("run RUN01 started"), "human line: {text:?}");
        assert!(
            !text.contains('{'),
            "text mode must not emit JSON: {text:?}"
        );
    }
}
