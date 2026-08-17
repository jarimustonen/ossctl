//! `ossctl release …` handlers.
//!
//! The full verb set (`plan`/`cut`/`resume`/`verify`/`show`/`list`/`abandon`) is
//! implemented here as a thin dispatcher over `ossctl-core::release`: the plan
//! seam, the phase-barrier coordinator, and the event-sourced journal all live in
//! the core crate; this module wires them to argument parsing, the journal-root
//! resolution, and the canonical `--json` envelope.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;

use ossctl_core::contract::schema::{Contract, Status};
use ossctl_core::contract::{self, LoadError, Normalized};
use ossctl_core::ports::GitRepo;
use ossctl_core::protocol::journal::{
    EventKind, JournalEvent, RunState, RunStatus, JOURNAL_SCHEMA_VERSION,
};
use ossctl_core::protocol::plan::ReleasePlan;
use ossctl_core::protocol::reconcile::ReconcileReport;
use ossctl_core::release::adapters::{verification_artifacts, EffectCtx, EMPTY_ARTIFACTS};
use ossctl_core::release::coordinator::{self, CutError, ProgressSink};
use ossctl_core::release::distribution::{find_undeclared_distribution, UndeclaredDistribution};
use ossctl_core::release::journal::{self, Journal, JournalPaths};

use crate::cli::ReleaseAction;
use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::{
    ReadOnlyJournalStore, RealClock, RealCommandRunner, RealFs, RealGitRepo, RealIdGen,
    RealJournalStore, RealRegistryQuery, RealTagger, StaleLockOutcome,
};

/// Arguments for `release plan`.
///
/// There is deliberately **no** `--version` input: the release version is derived
/// solely from the workspace manifest (the single source of truth), because a cut
/// publishes the version already in the tree — it does not bump it
/// (`release-drop-version-flag`). A stray `--version` fails at the clap boundary
/// (`unexpected argument`) rather than being silently ignored.
///
/// The one exception is the **opt-in** `--bump major|minor|patch`: an engine-owned
/// version bump where the human supplies only the semantic *level* and the engine
/// **computes** the new version from the current manifest version (still no
/// hand-typed literal — the single-source-version decision holds). Omitting `--bump`
/// is the default, unchanged, publish-the-tree-version path.
#[derive(Args, Debug)]
pub struct PlanArgs {
    /// Repository root to plan a release for (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Own the version bump: compute the new version from the current manifest
    /// version + this semantic level (`major` → X+1.0.0, `minor` → X.Y+1.0, `patch`
    /// → X.Y.Z+1) and seal a bump phase (version + intra-workspace pin rewrites +
    /// Cargo.lock refresh + CHANGELOG finalize + any declared `bump_hook`). Omit for
    /// the default path that publishes the version already in the manifest.
    #[arg(long, value_name = "LEVEL")]
    pub bump: Option<String>,
    /// Permit planning ossctl itself with a binary built from a different commit.
    /// This is an explicit escape hatch for a deliberate ossctl self-cut.
    #[arg(long)]
    pub allow_stale_binary: bool,
}

/// Arguments for `release cut`.
///
/// `release plan` is read-only and persists nothing, so `cut` re-derives the plan
/// from the *current* repo state (the version is derived from the workspace
/// manifest, the single source of truth), then refuses unless the recomputed
/// `plan_id` equals the approved `--plan` (drift check, ADR-0002 §3). Like `plan`,
/// there is **no** `--version` input (`release-drop-version-flag`).
#[derive(Args, Debug)]
pub struct CutArgs {
    /// The sealed plan id to execute (from `release plan`). The cut refuses if the
    /// current repository no longer hashes to it.
    #[arg(long, value_name = "PLAN_ID")]
    pub plan: String,
    /// Repository root to cut the release in (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Override the release-journal location (default: the repo's
    /// `git-common-dir/ossctl/releases`). For CI or debugging (ADR-0003 §3).
    #[arg(long, value_name = "DIR")]
    pub journal_dir: Option<PathBuf>,
    /// The same `--bump <level>` passed to `release plan`, when the sealed plan owns
    /// a version bump. The cut recomputes the new version from the current manifest
    /// version + this level and refuses (`plan_stale`) if the result does not hash to
    /// `--plan` — so a plan sealed as `--bump minor` cannot be cut as `major` (or
    /// without a bump). Omit for a `--bump`-less plan.
    #[arg(long, value_name = "LEVEL")]
    pub bump: Option<String>,
    /// Permit cutting ossctl itself with a binary built from a different commit.
    /// This is an explicit escape hatch for a deliberate ossctl self-cut.
    #[arg(long)]
    pub allow_stale_binary: bool,
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
    /// outage or failed destination observation). Without this the resume
    /// refuses on any `unknown` target rather than assume it did not publish
    /// (ADR-0003 §4). With it, an unverifiable target is trusted to the journal:
    /// a recorded publish is skipped, an unrecorded one is (re-)published.
    #[arg(long)]
    pub allow_unverified: bool,
}

/// Arguments for `release list` — the journal-location flags only (`list` takes
/// no run id: it enumerates every run under the journal root).
#[derive(Args, Debug)]
pub struct ListArgs {
    /// Repository whose release journal to enumerate (default: current
    /// directory). The journal is rooted at `<git-common-dir>/ossctl/releases`,
    /// so any linked worktree of the repo resolves to the same run set.
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// List the runs in this directory instead of resolving it from git
    /// (`<git-common-dir>/ossctl/releases`). For CI, tests, and post-mortem
    /// queries against an archived journal (needs no repository).
    #[arg(long, value_name = "PATH")]
    pub journal_dir: Option<PathBuf>,
}

/// Arguments for `release abandon`.
#[derive(Args, Debug)]
pub struct AbandonArgs {
    /// The run id to abandon.
    #[arg(value_name = "RUN_ID")]
    pub run_id: String,
    /// Why the run is being abandoned (journaled). Optional; a generic reason is
    /// recorded when omitted. `allow_hyphen_values` lets a reason begin with `--`
    /// (e.g. quoting a flag: `--reason "--no-verify insufficient"`) without clap
    /// mistaking the value for a flag.
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub reason: Option<String>,
    /// Repository whose release journal to write to (default: current directory).
    /// The journal is rooted at `<git-common-dir>/ossctl/releases`, so any linked
    /// worktree of the repo resolves to the same run state.
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Read/write the journal in this directory instead of resolving it from git
    /// (`<git-common-dir>/ossctl/releases`). For CI and tests.
    #[arg(long, value_name = "PATH")]
    pub journal_dir: Option<PathBuf>,
}

/// Dispatch a `release` subcommand to its handler.
pub fn dispatch(action: ReleaseAction, format: OutputFormat) -> Result<(), CliError> {
    match action {
        ReleaseAction::Plan(args) => plan(&args, format),
        ReleaseAction::Cut(args) => cut(&args, format),
        ReleaseAction::Resume(args) => resume(&args, format),
        ReleaseAction::Verify(args) => verify(&args, format),
        ReleaseAction::Show(args) => show(&args, format),
        ReleaseAction::List(args) => list(&args, format),
        ReleaseAction::Abandon(args) => abandon(&args, format),
    }
}

/// Check a self-cut's executable provenance against its release tree.
///
/// A release plan derives its version and targets from the live tree, so an ossctl
/// binary built from an older commit can otherwise produce a convincing *self-cut*
/// plan while running obsolete release-engine code. The commits of a downstream
/// release tree have no relationship to this binary, so this guard applies only
/// when that tree's `origin` identifies the canonical ossctl repository.
fn release_binary_warnings(
    git: &RealGitRepo,
    head_sha: &str,
    allow_stale_binary: bool,
) -> Result<Vec<String>, CliError> {
    let mut warnings = Vec::new();
    let is_ossctl_source_tree = git
        .origin_url()
        .ok()
        .is_some_and(|origin| is_ossctl_source_tree(&origin));
    if !is_ossctl_source_tree {
        return Ok(warnings);
    }
    if let Some(warning) =
        compiled_provenance_warning(true, crate::cli::GIT_COMMIT, head_sha, allow_stale_binary)?
    {
        warnings.push(warning);
    }

    match git.is_dirty() {
        Ok(true) => warnings.push(
            "the release tree has uncommitted changes to tracked files; the provenance check only evaluates HEAD, not uncommitted changes. Commit or discard them before cutting a release".to_string(),
        ),
        Ok(false) => {}
        Err(error) => warnings.push(format!(
            "could not determine whether the release tree is dirty ({error}); provenance only confirms HEAD"
        )),
    }

    Ok(warnings)
}

fn is_ossctl_source_tree(origin: &str) -> bool {
    ossctl_core::vcs::parse_github_slug(origin).is_some_and(|slug| {
        ossctl_core::vcs::parse_github_slug(crate::cli::SOURCE_REPOSITORY)
            .is_some_and(|source| slug.eq_ignore_ascii_case(&source))
    })
}

fn compiled_provenance_warning(
    is_ossctl_source_tree: bool,
    compiled_commit: &str,
    head_sha: &str,
    allow_stale_binary: bool,
) -> Result<Option<String>, CliError> {
    if !is_ossctl_source_tree {
        return Ok(None);
    }
    if !has_git_commit_provenance(compiled_commit) {
        return Err(CliError::user(
            "unverifiable_binary_provenance",
            "CANNOT VERIFY BINARY: this ossctl executable was built without git commit provenance, so it cannot safely cut ossctl itself. Build this ossctl checkout with `cargo build --release -p ossctl` before planning or cutting a self-release",
        ));
    }
    if compiled_commit.eq_ignore_ascii_case(head_sha) {
        return Ok(None);
    }

    let message = format!(
        "STALE BINARY: this ossctl executable was built from commit {compiled_commit}, but ossctl's release tree is at {head_sha}. Rebuild this ossctl checkout with `cargo build --release -p ossctl` before planning or cutting a self-release"
    );
    if allow_stale_binary {
        Ok(Some(format!(
            "{message}; proceeding only because --allow-stale-binary was passed"
        )))
    } else {
        Err(CliError::user("stale_binary", message))
    }
}

fn has_git_commit_provenance(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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
    ensure_single_distribution(&normalized.contract)?;

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
    let provenance_warnings = release_binary_warnings(&git, &head_sha, args.allow_stale_binary)?;

    let facts = ossctl_core::facts::gather(&root, &RealFs, &git);

    // Derive the release version from the workspace manifest (the single source of
    // truth). A cut publishes the version already in the manifest — it does not bump
    // it — so the version is a projection of the tree, never an independent input:
    // there is no `--version` flag, which removes the two-masters drift footgun at the
    // root (`release-drop-version-flag`, closing `release-cut-publish-noop`).
    // Derive the sealed plan — the manifest version for the default path, or the
    // computed new version + bump phase for `--bump <level>` (opt-in).
    let plan = derive_release_plan(
        &normalized.contract,
        &facts,
        &head_sha,
        args.bump.as_deref(),
    )?;
    let paths = JournalPaths::from_git(&git, None).map_err(|e| {
        CliError::system(
            "plan_store_unavailable",
            format!("cannot locate the durable plan store: {e}"),
        )
    })?;
    ossctl_core::release::plan_store::PlanStore::new(paths)
        .save(&plan, &normalized.contract)
        .map_err(plan_store_error)?;

    let mut warnings = normalized.problems.warnings.clone();
    warnings.extend(provenance_warnings);
    // Surface a non-blocking warning when the contract configures nothing to
    // publish — the plan would tag only.
    if plan.targets.is_empty() {
        warnings.push(
            "the contract declares no publish targets — this plan would create the git tag only"
                .to_string(),
        );
    }
    // A plan whose engine-published crate depends on a CI-delegated one can never be
    // cut (publish-all runs before the tag that triggers CI). Warn here — `cut`
    // refuses it — so the approver fixes the contract rather than discovering it
    // mid-cut.
    warnings.extend(ossctl_core::release::plan::delegated_dependency_messages(
        &ossctl_core::release::plan::delegated_dependency_conflicts(&plan, &facts),
    ));
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
    // A `--bump` plan owns an engine version bump the cut EXECUTES (edit the version +
    // pins + Cargo.lock + CHANGELOG in the clean checkout, run any bump_hook, commit, and
    // tag the bump commit). Say what will change so an approver reviews the whole effect.
    if let Some(bump) = &plan.bump {
        warnings.push(format!(
            "this plan owns an engine version bump ({} → {}): `release cut` will set the workspace \
             version, rewrite {} intra-workspace `=`-pin(s), refresh Cargo.lock, finalize the \
             CHANGELOG, run any bump_hook, commit, and tag that bump commit — all in a clean \
             checkout of the sealed commit, before any publish",
            bump.from_version,
            bump.to_version,
            bump.pin_rewrites.len(),
        ));
        // Surface any declared bump_hook VERBATIM: it is arbitrary code the cut runs during
        // the release (a supply-chain surface), so an approver must see exactly what will
        // run. Quoted to prevent output-spoofing via the value.
        if let Some(hook) = &bump.bump_hook {
            warnings.push(format!(
                "this bump declares a bump_hook the engine WILL RUN during the release (as `sh -c` \
                 in the clean checkout, with the cut's environment) — review it as trusted code: \
                 {hook:?}"
            ));
        }
    }

    match format {
        OutputFormat::Json => crate::output::emit_json(&plan, &warnings)?,
        OutputFormat::Text => render_plan_text(&plan, &warnings)?,
    }
    Ok(())
}

/// `ossctl release verify` — read-only reconcile of a journaled run against
/// remote registry state (ADR-0002 §1, ADR-0003 state table).
///
/// Reads the run's state straight from the authoritative event log (no lock, no
/// manifest self-heal, no writes) and dispatches each published or delegated
/// target through its real read-only destination check: registry index, Homebrew
/// formula fetch, or GitHub Release asset view. An observation that cannot be
/// performed degrades to `unknown`, never a false `missing`. Emits the report under the
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

    // Newer runs retain the authenticated plan beside the journal. It restores
    // exact adapters and platform obligations; older runs take the journal-only
    // compatibility path and still perform destination checks from their receipts.
    let plan = ossctl_core::release::plan_store::PlanStore::new(paths.clone())
        .load(&state.plan_id)
        .map_err(plan_store_error)?;
    let artifacts = plan
        .as_ref()
        .map_or_else(Default::default, verification_artifacts);
    let runner = RealCommandRunner;
    let clock = RealClock;
    let registry = RealRegistryQuery;
    let ctx = EffectCtx {
        runner: &runner,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
        artifacts: &artifacts,
    };
    let report = ossctl_core::release::reconcile::reconcile_with_plan(&state, plan.as_ref(), &ctx);
    let warnings = reconcile_warnings(&state, &report);

    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &warnings)?,
        OutputFormat::Text => render_reconcile_text(&report, &warnings)?,
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
        if report.targets.iter().any(|row| &row.target == target) {
            continue;
        }
        // A cancelled target has a known reason on the journal — report that,
        // rather than the generic "not yet published" that would misread an
        // intentional skip as an interruption.
        if let Some(reason) = state.cancelled.get(target) {
            warnings.push(format!("target '{target}' was cancelled: {reason}"));
        } else if state.delegated.contains(target) {
            // A CI-delegated target has no engine receipt by design — its artifact
            // is produced by the tag-triggered CI. Say so rather than misread it as
            // an interrupted publish.
            warnings.push(format!(
                "target '{target}' is CI-delegated (its artifact is produced by the \
                 tag-triggered release workflow, not the engine); it is not reconciled"
            ));
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

fn render_reconcile_text(report: &ReconcileReport, warnings: &[String]) -> Result<(), CliError> {
    crate::output::stdoutln!("run_id:     {}", report.run_id)?;
    crate::output::stdoutln!("plan_id:    {}", report.plan_id)?;
    crate::output::stdoutln!(
        "status:     {} (journal seq {})",
        report.run_status.as_str(),
        report.journal_seq
    )?;
    let s = &report.summary;
    crate::output::stdoutln!(
        "reconciled: {} ({} matches, {} conflicts, {} missing, {} unknown)",
        s.reconciled,
        s.matches,
        s.conflicts,
        s.missing,
        s.unknown
    )?;
    for t in &report.targets {
        crate::output::stdoutln!(
            "  {:<10} {:<8} {:<20} {}",
            t.target,
            t.ecosystem,
            format!("{}@{}", t.package.as_deref().unwrap_or("<none>"), t.version),
            t.outcome.as_str(),
        )?;
        if let Some(detail) = &t.detail {
            crate::output::stdoutln!("             └─ {detail}")?;
        }
    }
    for w in warnings {
        crate::output::stdoutln!("warning:    {w}")?;
    }
    Ok(())
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
    let paths = resolve_journal_paths(args.repo_root.as_ref(), args.journal_dir.as_deref())?;

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
        OutputFormat::Text => render_show_text(&state, window)?,
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
fn render_show_text(state: &RunState, events: &[JournalEvent]) -> Result<(), CliError> {
    crate::output::stdoutln!("run_id:     {}", state.run_id)?;
    crate::output::stdoutln!("plan_id:    {}", state.plan_id)?;
    crate::output::stdoutln!("version:    {}", state.version)?;
    match state.status {
        RunStatus::Abandoned => match &state.abandon_reason {
            Some(reason) => crate::output::stdoutln!(
                "status:     abandoned ({reason}) (journal seq {})",
                state.applied_seq
            )?,
            None => crate::output::stdoutln!(
                "status:     abandoned (journal seq {})",
                state.applied_seq
            )?,
        },
        status => {
            let phase = state
                .current_phase
                .map(|p| format!(" — in {}", p.as_str()))
                .unwrap_or_default();
            crate::output::stdoutln!(
                "status:     {}{phase} (journal seq {})",
                status.as_str(),
                state.applied_seq
            )?;
        }
    }

    crate::output::stdoutln!("targets:    {}", state.targets.len())?;
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
        crate::output::stdoutln!("  {target:<10} {landing}")?;
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
        if tstate.github_release_delegated {
            steps.push("release→CI");
        }
        crate::output::stdoutln!("tag {tag}: {}", steps.join(", "))?;
    }

    crate::output::stdoutln!("events:     {}", events.len())?;
    for event in events {
        crate::output::stdoutln!("  {}", render_event_line(event))?;
    }
    Ok(())
}

/// One run's summary row in `release list` — enough for the `/oss-release`
/// skill's in-flight-run gate ("is there an active run?") plus post-mortem
/// triage. Additive `--json` fields only (§10): a new column is appended, never a
/// rename/removal of one already published.
#[derive(serde::Serialize)]
struct RunSummary {
    /// The run's unique id (a ULID; lexicographically sortable by start time).
    run_id: String,
    /// Derived status wire string: `in_progress` / `completed` / `abandoned`.
    status: &'static str,
    /// The chosen release version this run publishes.
    version: String,
    /// The git tag this run creates (`v{version}`, the coordinator's tag phase).
    tag: String,
    /// The sealed, content-addressed plan id this run executes.
    plan_id: String,
    /// `true` while the run is neither completed nor abandoned — the gate an agent
    /// keys on to refuse sealing a second plan while one is live.
    in_flight: bool,
    /// Unix timestamp of the `RunCreated` event (run start).
    started_ts: u64,
    /// Unix timestamp of the most recently applied event.
    updated_ts: u64,
    /// The recorded abandon reason when the run was abandoned; `null` otherwise.
    /// Always serialized (never skipped) so the run's wire shape is the same for
    /// every status — a consumer reads one document shape, per the codebase's
    /// Option-serializes-null convention (`RunState::abandon_reason`).
    abandon_reason: Option<String>,
}

impl RunSummary {
    fn from_state(state: &RunState) -> Self {
        let terminal = matches!(state.status, RunStatus::Completed | RunStatus::Abandoned);
        Self {
            run_id: state.run_id.clone(),
            status: state.status.as_str(),
            version: state.version.clone(),
            tag: format!("v{}", state.version),
            plan_id: state.plan_id.clone(),
            in_flight: !terminal,
            started_ts: state.created_ts,
            updated_ts: state.updated_ts,
            abandon_reason: state.abandon_reason.clone(),
        }
    }
}

/// The `data` body of `release list`: the run set plus a pre-computed count of the
/// in-flight ones, so an agent's "already-active run?" gate is a single field read
/// rather than a client-side filter it might get wrong.
///
/// **Gate contract:** it is safe to seal/start a new run **only** when
/// `in_flight_count == 0` **and** `unreadable` is empty. An unreadable run is
/// counted as uncertainty, never as "not in flight" — a corrupt or too-new journal
/// that happened to be an active run must not silently read as a clear coast.
#[derive(serde::Serialize)]
struct RunListBody {
    /// Every readable run under the journal root, in deterministic order (by start
    /// time, then run id). An empty list is a normal result, not an error.
    runs: Vec<RunSummary>,
    /// How many of [`Self::runs`] are still in flight (`in_progress`).
    in_flight_count: usize,
    /// Run ids whose journal could **not** be read (corrupt, or written by a newer
    /// ossctl). Surfaced explicitly rather than dropped: one of these could be an
    /// active run, so the in-flight gate must treat a non-empty `unreadable` as
    /// "cannot be certain", not as zero active runs. Each also has a `warnings`
    /// entry with the underlying error.
    unreadable: Vec<String>,
}

/// `ossctl release list` — enumerate every release run under the journal root with
/// its status (ADR-0003 §3), for the `/oss-release` skill's in-flight-run gate.
///
/// Reads each run's state authoritatively from its event log (never the possibly-
/// stale manifest cache) through the read-only journal store — no lock, no writes.
/// The set is sorted deterministically by start time (then run id) so the output
/// is stable across invocations. An empty journal root is a normal empty list, not
/// an error. A single unreadable run (a corrupt or too-new journal) is surfaced as
/// a warning and skipped rather than failing the whole enumeration — one bad
/// journal must not blind the gate to the other runs.
pub fn list(args: &ListArgs, format: OutputFormat) -> Result<(), CliError> {
    let paths = resolve_journal_paths(args.repo_root.as_ref(), args.journal_dir.as_deref())?;

    let store = ReadOnlyJournalStore;
    let run_ids = journal::list_runs(&store, &paths).map_err(|e| {
        CliError::system(
            "journal_error",
            format!(
                "cannot enumerate release runs under {}: {e}",
                paths.releases_dir().display()
            ),
        )
    })?;

    let mut runs = Vec::with_capacity(run_ids.len());
    let mut unreadable = Vec::new();
    let mut warnings = Vec::new();
    for run_id in run_ids {
        match journal::read_run_state(&store, &paths, &run_id) {
            Ok(Some(state)) => runs.push(RunSummary::from_state(&state)),
            // `list_runs` only returns ids with a non-empty journal, so an empty
            // read here is a benign race (the run was removed mid-list); skip it.
            Ok(None) => {}
            // A corrupt or too-new journal is NOT silently dropped: it is recorded
            // under `unreadable` (with a warning) so the in-flight gate cannot read
            // a clear coast when one of these might be an active run.
            Err(e) => {
                warnings.push(format!("run '{run_id}' could not be read: {e}"));
                unreadable.push(run_id);
            }
        }
    }
    // Deterministic order: by start time, then run id as a stable tiebreak for two
    // runs sharing a timestamp. `unreadable` is already sorted (list_runs sorts).
    runs.sort_by(|a, b| {
        a.started_ts
            .cmp(&b.started_ts)
            .then_with(|| a.run_id.cmp(&b.run_id))
    });

    let in_flight_count = runs.iter().filter(|r| r.in_flight).count();
    // The single-active-cut lock permits at most one in-flight run per repo; more
    // than one means the invariant was violated (a lock bypass or journal
    // corruption) — surface it rather than letting it read as a benign count.
    if in_flight_count > 1 {
        warnings.push(format!(
            "{in_flight_count} runs are in flight, but the single-active-cut invariant permits at \
             most one — the release journal may be corrupt or a lock was bypassed"
        ));
    }
    let body = RunListBody {
        runs,
        in_flight_count,
        unreadable,
    };

    match format {
        OutputFormat::Json => crate::output::emit_json(&body, &warnings)?,
        OutputFormat::Text => render_list_text(&body, &warnings)?,
    }
    Ok(())
}

/// Render the run list as a human table (text mode).
fn render_list_text(body: &RunListBody, warnings: &[String]) -> Result<(), CliError> {
    if body.runs.is_empty() && body.unreadable.is_empty() {
        crate::output::stdoutln!("no release runs found")?;
    } else if !body.runs.is_empty() {
        crate::output::stdoutln!(
            "{} run(s), {} in flight",
            body.runs.len(),
            body.in_flight_count
        )?;
        for r in &body.runs {
            let flight = if r.in_flight { " *" } else { "  " };
            crate::output::stdoutln!(
                "{flight} {:<28} {:<12} {:<10} plan {}",
                r.run_id,
                r.status,
                r.tag,
                short_sha(&r.plan_id),
            )?;
            if let Some(reason) = &r.abandon_reason {
                crate::output::stdoutln!("     └─ abandoned: {reason}")?;
            }
        }
    }
    if !body.unreadable.is_empty() {
        crate::output::stdoutln!(
            "unreadable ({}): {} — status unknown, may be active",
            body.unreadable.len(),
            body.unreadable.join(", ")
        )?;
    }
    for w in warnings {
        crate::output::stdoutln!("warning: {w}")?;
    }
    Ok(())
}

/// The generic reason recorded when `release abandon` is run without `--reason`.
const DEFAULT_ABANDON_REASON: &str = "abandoned by operator (no reason given)";

/// The `data` body of `release abandon`: what the run looked like at abandonment,
/// with the irreversibility semantics made explicit.
#[derive(serde::Serialize)]
struct AbandonReport {
    /// The abandoned run's id.
    run_id: String,
    /// Always `abandoned` — the run's new terminal status.
    status: &'static str,
    /// The recorded abandon reason (the supplied `--reason`, or the default).
    reason: String,
    /// The version the run was cutting.
    version: String,
    /// Targets that were **already published** under this run before abandonment.
    /// They remain published — abandoning records that the operator is giving up
    /// on the run, it does **not** (and cannot) undo an irreversible publish
    /// (ADR-0003 §4). Empty when nothing had landed yet.
    published_targets: Vec<String>,
    /// The explicit semantics of this abandonment, so a caller never reads it as a
    /// rollback.
    note: &'static str,
}

/// `ossctl release abandon <run_id>` — terminally mark a non-terminal run
/// un-resumable by appending a `run_abandoned` event to its journal (ADR-0002 /
/// ADR-0003).
///
/// Event-sourced: history is **appended to, never rewritten or deleted** — the
/// abandonment is one more fact on the durable log, after which the reducer
/// freezes the projection (a later stray event cannot un-abandon the run). Opens
/// the run under the single-active-cut lock (so it never races a live cut/resume),
/// refuses a run that is already terminal (a `completed` run succeeded; an
/// `abandoned` run stays abandoned) with a distinct, caller-fixable error, and —
/// critically — does **not** roll anything back: any target already published
/// under this run stays published, and the output says so explicitly.
pub fn abandon(args: &AbandonArgs, format: OutputFormat) -> Result<(), CliError> {
    // Validate/normalize the reason up front, before any I/O. An *absent* flag falls
    // back to the generic default; a *provided* reason is trimmed and must be
    // non-blank, control-character-free, and bounded — it is journaled durably and
    // rendered on one line by `show`/`list`, so a newline or a megabyte of text
    // would pollute every later read.
    let reason = normalize_reason(args.reason.as_deref())?;

    let paths = resolve_journal_paths(args.repo_root.as_ref(), args.journal_dir.as_deref())?;

    let store = RealJournalStore;
    let clock = RealClock;
    let (mut journal, stale_lock_warning) =
        open_abandon_journal(&store, &clock, paths, &args.run_id)?;

    // A terminal run cannot be abandoned: recording a second terminal fact would be
    // meaningless. Distinguish the two cases so the caller gets an actionable code.
    match journal.state().status {
        RunStatus::Completed => {
            return Err(CliError::user(
                "run_completed",
                format!(
                    "run {} already completed successfully — there is nothing to abandon",
                    args.run_id
                ),
            )
            .with_invalid_value(args.run_id.clone()));
        }
        RunStatus::Abandoned => {
            return Err(CliError::user(
                "run_already_abandoned",
                format!(
                    "run {} was already abandoned{} — it stays abandoned",
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

    // Snapshot what already landed *before* appending — these publishes survive the
    // abandonment (it is not a rollback), and the report names them.
    let published_targets: Vec<String> = journal.state().published.keys().cloned().collect();
    let version = journal.state().version.clone();

    let state = journal
        .append(EventKind::RunAbandoned {
            reason: reason.clone(),
        })
        .map_err(|e| {
            CliError::system(
                "journal_error",
                format!(
                    "run {}: could not journal the abandonment: {e} — the run may be in an unknown \
                     state",
                    args.run_id
                ),
            )
        })?;
    debug_assert_eq!(state.status, RunStatus::Abandoned);

    // Everything the report needs is already captured (snapshots above + the append
    // result). Drop the journal now to release the single-active-cut lock *before*
    // rendering — a blocked stdout pipe must never hold the global release lock.
    drop(journal);

    let report = AbandonReport {
        run_id: args.run_id.clone(),
        status: RunStatus::Abandoned.as_str(),
        reason,
        version,
        published_targets: published_targets.clone(),
        note: "the run is marked abandoned and cannot be resumed; abandoning does NOT undo any \
               publish that already landed — reconcile or yank those manually if needed",
    };

    // Surface already-published targets as a warning too: they are the one thing an
    // operator abandoning a run most needs to be reminded still exists remotely.
    let mut warnings = Vec::new();
    if let Some(warning) = stale_lock_warning {
        warnings.push(warning);
    }
    if !published_targets.is_empty() {
        warnings.push(format!(
            "{} target(s) were already published under this run and remain published \
             (abandon does not roll back): {}",
            published_targets.len(),
            published_targets.join(", ")
        ));
    }

    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &warnings)?,
        OutputFormat::Text => render_abandon_text(&report, &warnings)?,
    }
    Ok(())
}

/// Open a journal for abandonment. `abandon` alone may recover a stale `O_EXCL`
/// lock: its recorded hostname must match and `kill -0` must prove the PID dead.
/// It removes that lock then retries acquisition exactly once.
fn open_abandon_journal<'a>(
    store: &'a RealJournalStore,
    clock: &'a RealClock,
    paths: JournalPaths,
    run_id: &str,
) -> Result<(Journal<'a>, Option<String>), CliError> {
    match Journal::open(store, clock, paths.clone(), run_id) {
        Ok(journal) => Ok((journal, None)),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            match RealJournalStore::break_stale_lock(&paths.lock_file()) {
                Ok(StaleLockOutcome::Broken { pid }) => {
                    let warning = format!(
                        "broke stale single-active-cut lock held by pid {pid}: kill -0 reported ESRCH (the holder no longer exists)"
                    );
                    let journal = Journal::open(store, clock, paths, run_id)
                        .map_err(|retry_error| abandon_open_error(run_id, retry_error))?;
                    Ok((journal, Some(warning)))
                }
                Ok(StaleLockOutcome::NotBroken { reason }) => {
                    Err(abandon_lock_not_broken_error(run_id, reason))
                }
                Err(inspect_error) => Err(abandon_lock_not_broken_error(
                    run_id,
                    format!("the lock could not be inspected: {inspect_error}"),
                )),
            }
        }
        Err(error) => Err(abandon_open_error(run_id, error)),
    }
}

/// The longest `--reason` the abandon command accepts. A reason is a short
/// operator note journaled durably, not a document; a bound keeps a mistaken
/// megabyte-paste out of the permanent log.
const MAX_REASON_LEN: usize = 2048;

/// Validate and normalize the `--reason` value: `None` (flag absent) → the generic
/// default; `Some` → trimmed, and rejected if blank, containing control characters,
/// or over [`MAX_REASON_LEN`]. Pure and I/O-free so it fails fast before the journal
/// is even located.
fn normalize_reason(reason: Option<&str>) -> Result<String, CliError> {
    let Some(raw) = reason else {
        return Ok(DEFAULT_ABANDON_REASON.to_string());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::user(
            "invalid_reason",
            "the abandon reason must not be blank — omit --reason for the default, or give a real \
             reason",
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(CliError::user(
            "invalid_reason",
            "the abandon reason must not contain control characters (newlines, tabs, …) — it is \
             journaled and rendered on one line",
        )
        .with_invalid_value(raw.to_string()));
    }
    if trimmed.len() > MAX_REASON_LEN {
        return Err(CliError::user(
            "invalid_reason",
            format!(
                "the abandon reason is {} bytes; keep it under {MAX_REASON_LEN}",
                trimmed.len()
            ),
        ));
    }
    Ok(trimmed.to_string())
}

/// Map a `Journal::open` failure for `abandon` to the §10 envelope. A normal
/// held lock reaches this only after the one permitted stale-lock retry.
fn abandon_open_error(run_id: &str, e: std::io::Error) -> CliError {
    if e.kind() == std::io::ErrorKind::WouldBlock {
        return abandon_lock_not_broken_error(
            run_id,
            "the lock remained held after the stale-lock recovery attempt",
        );
    }
    open_run_error(run_id, e)
}

/// The actionable held-lock refusal, including the exact reason `abandon` did
/// not break it. This stays a caller-fixable §10 envelope: wait for a live cut,
/// or inspect the lock before attempting any manual recovery.
fn abandon_lock_not_broken_error(run_id: &str, reason: impl AsRef<str>) -> CliError {
    CliError::user(
        "cut_in_progress",
        format!(
            "cannot abandon run {run_id}: the single-active-cut lock is held and was not broken \
             because {}. If a `release cut`/`resume` is genuinely running, let it finish; otherwise \
             inspect `<git-common-dir>/ossctl/releases/.lock` before recovery.",
            reason.as_ref()
        ),
    )
    .with_invalid_value(run_id.to_string())
}

/// Render the abandonment outcome as human lines (text mode).
fn render_abandon_text(report: &AbandonReport, warnings: &[String]) -> Result<(), CliError> {
    crate::output::stdoutln!("run {} abandoned", report.run_id)?;
    crate::output::stdoutln!("version: {}", report.version)?;
    crate::output::stdoutln!("reason:  {}", report.reason)?;
    if report.published_targets.is_empty() {
        crate::output::stdoutln!("published: none (nothing had landed)")?;
    } else {
        crate::output::stdoutln!(
            "published (still live): {}",
            report.published_targets.join(", ")
        )?;
    }
    crate::output::stdoutln!("note: {}", report.note)?;
    for w in warnings {
        crate::output::stdoutln!("warning: {w}")?;
    }
    Ok(())
}

/// Resolve the release-journal paths for a pure journal operation
/// (`list`/`show`/`abandon`): an explicit `--journal-dir` is used verbatim and
/// needs neither a repository nor a valid cwd (a post-mortem query or an archived
/// journal must work from anywhere); otherwise the root is resolved from git as
/// `<git-common-dir>/ossctl/releases`.
fn resolve_journal_paths(
    repo_root: Option<&PathBuf>,
    journal_dir: Option<&Path>,
) -> Result<JournalPaths, CliError> {
    if let Some(dir) = journal_dir {
        return Ok(JournalPaths::new(dir));
    }
    let repo_root = resolve_repo_root(repo_root)?;
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
    })
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
                crate::output::stdoutln!(
                    "run {} is already complete — nothing to resume",
                    args.run_id
                )?;
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

    // Newer plans are durable: execution uses the sealed checkout, so a code fix
    // after a failed cut must not invalidate the already-approved plan.
    let plan = match ossctl_core::release::plan_store::PlanStore::new(journal.paths().clone())
        .load(&journal.state().plan_id)
        .map_err(plan_store_error)?
    {
        Some(plan) => plan,
        None => derive_resume_plan(&root, &git, journal.state(), &args.run_id)?,
    };

    let verification_artifacts = verification_artifacts(&plan);
    let ctx = EffectCtx {
        runner: &runner,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
        artifacts: &verification_artifacts,
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
                render_cut_success(&args.run_id, &plan)?;
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
    ensure_single_distribution(&normalized.contract)?;
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

    // `release resume` of an interrupted `--bump` run is not yet supported. Reconstructing
    // the exact sealed plan requires the contract + facts as they were at the SEALED
    // pre-bump commit, but the bump commit has moved HEAD (and possibly the pins) past it,
    // so re-deriving from the live tree would either mismatch `plan_id` or reconstruct a
    // wrong plan. Rather than risk a mis-resume on the irreversible cut path, refuse with a
    // clear, actionable message — the operator abandons and re-plans (a fresh cut
    // re-materializes a clean checkout and re-applies the bump). Full bump-run resume is a
    // documented follow-up gated behind the live acceptance cut
    // (`release-rust-workspace-multicrate`).
    if state.bump_inputs.is_some() {
        return Err(CliError::user(
            "resume_bump_unsupported",
            format!(
                "run {run_id} is an engine `--bump` run; resuming an interrupted bump cut is not \
                 yet supported (the bump commit moved HEAD past the sealed commit, so the sealed \
                 plan cannot be safely reconstructed from the live tree). Abandon it \
                 (`ossctl release abandon {run_id}`) and plan + cut a fresh release — a new cut \
                 re-applies the bump from a clean checkout."
            ),
        )
        .with_invalid_value(run_id.to_string()));
    }

    // Confirm the journal's sealed version still matches the CURRENT tree manifest
    // (single source of truth), exactly as `cut` derives it. A resume re-derives the
    // plan from live repo state, and a manifest-version edit between the failed cut
    // and its resume does NOT change `plan_id` (manifest versions are not part of the
    // content address), so the drift check below would not catch it — a resume could
    // otherwise publish the new manifest version while threading the journal's sealed
    // version into every probe/wait/receipt, the exact `release-cut-publish-noop`
    // mismatch. Deriving the tree version and comparing it to the sealed version
    // surfaces such an edit as a `resume_version_drift`.
    let resolved =
        ossctl_core::release::plan::resolve_release_version(&normalized.contract, &facts)
            .map_err(|e| resume_version_error(state, e))?;
    if resolved != state.version {
        return Err(resume_version_drift_error(state, &resolved));
    }
    // The journal is local mutable state, not trusted input — re-validate the sealed
    // version's shape before it becomes the `v{version}` tag (parity with plan/cut).
    validate_version(&resolved)?;
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
             will not continue a different plan under this run. Runs planned by this ossctl version or \
             later persist their sealed plan and resume across code fixes; this run has no stored plan.",
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
/// `ossctl release cut --plan <id>` — execute a sealed plan across the
/// phase-barrier coordinator, refusing on repo drift (ADR-0002 §2/§3).
///
/// Flow: re-derive the plan from the *current* contract + facts + `HEAD` and the
/// manifest-derived version (the single source of truth, no `--version` input);
/// **refuse (`plan_stale`) unless the recomputed `plan_id` equals `--plan`** (the
/// drift check — a commit, contract edit, or manifest-version change since `release
/// plan` aborts here rather than publishing something the human did not approve). On
/// acceptance, create the journalled run (single
/// active cut) and drive dry-run-all → build-all → publish-all → tag-once through
/// the coordinator, streaming each journalled fact.
///
/// Output (§12): a `--output=jsonl`-style event stream — with `--json`, one
/// [`JournalEvent`] per line; otherwise human progress lines. `cut` never emits a
/// single `--json` envelope (a partially-irreversible, streaming command).
///
/// On any phase failure the run **stops with no rollback**; what landed is
/// journalled and the error names the `run_id` for `release verify` / `resume`.
#[allow(clippy::too_many_lines)] // Coordinates stored-plan and distribution preflights before journal creation.
pub fn cut(args: &CutArgs, format: OutputFormat) -> Result<(), CliError> {
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
    ensure_single_distribution(&normalized.contract)?;
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
    let provenance_warnings = release_binary_warnings(&git, &head_sha, args.allow_stale_binary)?;
    let facts = ossctl_core::facts::gather(&root, &RealFs, &git);
    ensure_declared_distribution(&normalized.contract, &facts)?;

    // Resolve the sibling plan store before choosing the bump disposition. A stored
    // plan is authoritative; legacy/machine-missing plans retain flag-driven fallback.
    let paths = JournalPaths::from_git(&git, args.journal_dir.as_deref()).map_err(|e| {
        CliError::system(
            "io_error",
            format!("cannot resolve the release-journal directory: {e}"),
        )
    })?;
    let stored = ossctl_core::release::plan_store::PlanStore::new(paths.clone())
        .load(&args.plan)
        .map_err(plan_store_error)?;
    let stored_bump = stored
        .as_ref()
        .and_then(|p| p.bump.as_ref())
        .map(|b| b.level.as_str());
    if let (Some(stored_level), Some(flag)) = (stored_bump, args.bump.as_deref()) {
        if stored_level != flag {
            return Err(CliError::user("bump_mismatch", format!("stored plan was sealed with --bump {stored_level}, but release cut received --bump {flag}")));
        }
    }
    let current = derive_release_plan(
        &normalized.contract,
        &facts,
        &head_sha,
        stored_bump.or(args.bump.as_deref()),
    )?;
    if current.plan_id != args.plan {
        return Err(plan_stale_error(&args.plan, &current, stored.as_ref()));
    }

    // Preflight the plan *before* creating a run, so an unexecutable plan (an
    // unresolved package, a duplicate-ecosystem target) is refused up front rather
    // than leaving an orphaned `run_created` run behind.
    coordinator::validate_plan(&current).map_err(|e| cut_error_to_cli("(not created)", e))?;
    // …including the phase-ordering conflict a mixed engine/CI-published workspace can
    // declare: publish-all runs BEFORE the tag that triggers the delegated publish, so
    // an engine-published crate depending on a CI-delegated one can never be satisfied.
    // No retry or resume fixes it — only a contract edit — so refuse before the run.
    ensure_no_delegated_dependency_conflict(&current, &facts)?;

    for warning in provenance_warnings {
        eprintln!("warning: {warning}");
    }

    let store = RealJournalStore;
    let clock = RealClock;
    let idgen = RealIdGen;
    let runner = RealCommandRunner;
    let registry = RealRegistryQuery;
    let tagger = RealTagger::new(&root);

    // Create the run under the single-active-cut lock (RunCreated is journalled).
    // Derive the per-target journal ids through the same shared helper the
    // coordinator keys its facts by, so `RunCreated.targets` matches the
    // `published`/`built`/`dry_run` keys even when an ecosystem carries several
    // targets (two crates.io crates, or a crate plus its gh-releases/homebrew
    // channels — all under `rust`).
    let target_ids = ossctl_core::release::journal_target_ids(&current.targets);
    let mut journal = create_run_journal(&store, &clock, &idgen, paths, &current, target_ids)?;
    let run_id = journal.run_id().to_string();

    let mut sink = StreamSink::new(std::io::stdout(), matches!(format, OutputFormat::Json));
    // Emit the run identity first so the stream is self-contained.
    stream_run_created(&mut sink, &journal, &current);

    let ctx = EffectCtx {
        runner: &runner,
        clock: &clock,
        registry: &registry,
        repo_root: &root,
        artifacts: &EMPTY_ARTIFACTS,
    };

    match coordinator::execute(&mut journal, &current, &ctx, &tagger, &mut sink) {
        Ok(()) => {
            if !matches!(format, OutputFormat::Json) {
                render_cut_success(&run_id, &current)?;
            }
            Ok(())
        }
        Err(e) => Err(cut_error_to_cli(&run_id, e)),
    }
}

/// Create the run journal for a cut: a `--bump` run persists the sealed `head_sha` + bump
/// inputs (`Journal::create_bump`) so `release resume` can reconstruct the exact sealed
/// plan against the pre-bump commit after the bump commit moves HEAD
/// (`release-rust-workspace-multicrate`); a no-bump run uses plain `Journal::create`.
fn create_run_journal<'a>(
    store: &'a RealJournalStore,
    clock: &'a RealClock,
    idgen: &'a RealIdGen,
    paths: JournalPaths,
    current: &ReleasePlan,
    target_ids: Vec<String>,
) -> Result<Journal<'a>, CliError> {
    match &current.bump {
        Some(bump) => Journal::create_bump(
            store,
            clock,
            idgen,
            paths,
            current.plan_id.clone(),
            current.version.clone(),
            target_ids,
            current.head_sha.clone(),
            ossctl_core::protocol::journal::BumpInputs {
                level: bump.level.as_str().to_string(),
                from_version: bump.from_version.clone(),
            },
        ),
        None => Journal::create(
            store,
            clock,
            idgen,
            paths,
            current.plan_id.clone(),
            current.version.clone(),
            target_ids,
        ),
    }
    .map_err(create_journal_error)
}

/// The §10 `plan_stale` refusal: the current repo no longer hashes to the
/// approved plan (ADR-0002 §3). Exit 1 — caller-fixable by re-planning.
fn plan_stale_error(
    approved: &str,
    current: &ReleasePlan,
    stored: Option<&ReleasePlan>,
) -> CliError {
    let difference = match stored {
        Some(plan) if plan.head_sha != current.head_sha => format!(
            "HEAD moved from {} to {}",
            short_sha(&plan.head_sha),
            short_sha(&current.head_sha)
        ),
        Some(plan) if plan.version != current.version => format!(
            "manifest version changed from {} to {}",
            plan.version, current.version
        ),
        Some(_) => "the contract or detected facts changed".to_string(),
        None => "the current repository differs from the plan".to_string(),
    };
    CliError::user(
        "plan_stale",
        format!(
            "the approved plan is stale: {difference}. Re-run `ossctl release plan` (with `--bump` if intended) and approve what it prints",
        ),
    )
    .with_invalid_value(approved.to_string())
    .with_expected(serde_json::json!({ "recomputed_plan_id": current.plan_id }))
}

fn plan_store_error(error: ossctl_core::release::plan_store::PlanStoreError) -> CliError {
    match error {
        ossctl_core::release::plan_store::PlanStoreError::Corrupt { plan_id, detail } => {
            CliError::system(
                "plan_store_corrupt",
                format!("stored plan {plan_id} is corrupt: {detail}"),
            )
        }
        other => CliError::system("plan_store_error", other.to_string()),
    }
}

/// Derive and seal the release plan the way `plan` and `cut` both need it: the
/// manifest version for the default path, or the computed new version + a sealed bump
/// phase for `--bump <level>`.
///
/// Shared by `plan` (to emit the sealed artifact) and `cut` (to recompute it for the
/// drift check), so both agree on how `--bump` maps to a plan down to the byte. The
/// core constructor owns the version arithmetic (a non-semver manifest version fails
/// closed as `unbumpable_version`), and the resulting tag-shape is validated for both
/// the current and the computed version.
fn derive_release_plan(
    contract: &Contract,
    facts: &ossctl_core::protocol::facts::Facts,
    head_sha: &str,
    bump_arg: Option<&str>,
) -> Result<ReleasePlan, CliError> {
    let current_version = resolve_version(contract, facts)?;
    // The current/derived version becomes the `v{version}` git tag — validate its shape
    // even though it came from the manifest (a manifest could carry a tag-unsafe string).
    validate_version(&current_version)?;
    match parse_bump_level(bump_arg)? {
        Some(level) => {
            // The core constructor OWNS the arithmetic (computes to_version from
            // level+from_version), so the plan can never seal a version that contradicts
            // its declared level. A non-semver manifest version fails closed here.
            let plan = ossctl_core::release::plan::build_with_bump(
                contract,
                facts,
                head_sha,
                &current_version,
                level,
            )
            .map_err(|e| bump_error_to_cli(level, e))?;
            // The computed version becomes the `v{version}` tag — validate its shape.
            validate_version(&plan.version)?;
            Ok(plan)
        }
        None => Ok(ossctl_core::release::plan::build(
            contract,
            facts,
            head_sha,
            &current_version,
        )),
    }
}

/// Resolve the release version from the workspace manifest (the single source of
/// truth) and map any failure to the §10 error envelope.
///
/// Thin wrapper over [`ossctl_core::release::plan::resolve_release_version`] used by
/// `plan`/`cut`/`resume` so all three share one derivation + one error surface.
fn resolve_version(
    contract: &Contract,
    facts: &ossctl_core::protocol::facts::Facts,
) -> Result<String, CliError> {
    ossctl_core::release::plan::resolve_release_version(contract, facts)
        .map_err(version_resolve_error)
}

/// Map a [`VersionResolveError`](ossctl_core::release::plan::VersionResolveError) to
/// the §10 error envelope with an actionable, operator-facing message.
///
/// `ossctl release cut` publishes the version already in the manifest — it does not
/// bump it — so the version is derived **solely** from the manifest (the single source
/// of truth); there is no `--version` input (`release-drop-version-flag`). Each
/// failure mode gets a distinct, fixable message.
fn version_resolve_error(err: ossctl_core::release::plan::VersionResolveError) -> CliError {
    use ossctl_core::release::plan::VersionResolveError;
    match err {
        // Fail CLOSED: a manifest-versioned target (npm/PyPI/…) whose version the
        // detector could not read — `version-source-fail-closed-nonrust`. A
        // distribution target (homebrew/binary) is skipped by design; a manifest
        // target with no readable version is a bug that must not publish blind.
        VersionResolveError::MissingManifestVersion { targets } => CliError::user(
            "version_source_unreadable",
            format!(
                "these manifest-versioned target(s) have no readable manifest version, so the \
                 release version cannot be confirmed for them: {}. These ecosystems ARE \
                 manifest-versioned (unlike a homebrew/binary distribution target, which is \
                 skipped by design), so ossctl fails closed rather than publish an unchecked \
                 version. Ensure each package's manifest declares a version the detector can read \
                 (`ossctl facts --json` shows what was detected).",
                render_unversioned_rows(&targets)
            ),
        ),
        VersionResolveError::InconsistentTree { versions } => CliError::user(
            "version_inconsistent_tree",
            format!(
                "the workspace manifests declare more than one version, so there is no single \
                 release version to derive: {}. Bring every publishable crate to the same version \
                 (a lockstep bump) in a release commit before planning/cutting.",
                render_version_rows(&versions)
            ),
        ),
        VersionResolveError::Undeterminable => CliError::user(
            "version_undeterminable",
            "no manifest version could be detected for any target, so the release version cannot \
             be derived. The version comes solely from the workspace manifest — ensure a \
             publishable package's manifest declares a version.",
        ),
    }
}

/// Render `<package> (<ecosystem>) is at <version>` rows as one comma-joined line —
/// the shared detail body for the tree-inconsistency error message.
fn render_version_rows(rows: &[ossctl_core::release::plan::VersionMismatch]) -> String {
    rows.iter()
        .map(|m| {
            format!(
                "{} ({}) is at {}",
                m.package,
                m.ecosystem.as_str(),
                m.manifest_version
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render `<package> (<ecosystem> → <registry>)` rows as one comma-joined line — the
/// detail body for the fail-closed `version_source_unreadable` error.
fn render_unversioned_rows(rows: &[ossctl_core::release::plan::UnversionedTarget]) -> String {
    rows.iter()
        .map(|t| {
            format!(
                "{} ({} → {})",
                t.package,
                t.ecosystem.as_str(),
                t.registry.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Map a version-resolution failure encountered during `release resume` to a
/// resume-appropriate §10 error.
///
/// Reached when the tree can no longer produce **any** release version (a manifest
/// version went unreadable, the tree became self-inconsistent, or no manifest version
/// remains). The version comes from the journal, so the fix is to restore the sealed
/// tree, not to derive a new one.
fn resume_version_error(
    state: &ossctl_core::protocol::journal::RunState,
    err: ossctl_core::release::plan::VersionResolveError,
) -> CliError {
    use ossctl_core::release::plan::VersionResolveError;
    let detail = match &err {
        VersionResolveError::MissingManifestVersion { targets } => render_unversioned_rows(targets),
        VersionResolveError::InconsistentTree { versions } => render_version_rows(versions),
        VersionResolveError::Undeterminable => "no manifest version could be detected".to_string(),
    };
    CliError::user(
        "resume_version_drift",
        format!(
            "run {} was sealed at version {}, but the tree manifest can no longer produce that \
             version: {detail}. A manifest edit occurred after the cut (a manifest edit does not \
             move the plan_id, so this is the check that catches it). Restore the sealed tree (a \
             clean checkout of the sealed commit), or plan and cut a new release — ossctl will not \
             resume a run under a different version than it was sealed with.",
            state.run_id, state.version
        ),
    )
    .with_invalid_value(state.run_id.clone())
}

/// The `resume_version_drift` refusal for the common case: the tree still resolves to
/// a single manifest version, but a *different* one than the run was sealed with (a
/// lockstep re-bump after the cut). A manifest-version edit does not move the
/// content-addressed `plan_id`, so this explicit comparison is the check that catches
/// it — the fix is to restore the sealed version, not to resume under a new one.
fn resume_version_drift_error(
    state: &ossctl_core::protocol::journal::RunState,
    tree_version: &str,
) -> CliError {
    CliError::user(
        "resume_version_drift",
        format!(
            "run {} was sealed at version {}, but the tree manifest is now at {tree_version}. A \
             manifest-version edit occurred after the cut (a manifest edit does not move the \
             plan_id, so this is the check that catches it). Restore the sealed version (a clean \
             checkout of the sealed commit), or plan and cut a new release — ossctl will not \
             resume a run under a different version than it was sealed with.",
            state.run_id, state.version
        ),
    )
    .with_invalid_value(state.run_id.clone())
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
        CutError::Checkout(_) => {
            // Fail-closed before any effect: the sealed commit could not be checked
            // out to publish from (typically not committed/pushed). Caller-fixable —
            // nothing external happened. `err`'s Display carries the full guidance.
            CliError::user("sealed_commit_unavailable", format!("run {run_id}: {err}"))
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
            head_sha: state.head_sha.clone(),
            bump: state.bump_inputs.clone(),
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
        EventKind::BumpApplied {
            commit,
            effective_date,
        } => format!(
            "  version bumped (commit {}, {effective_date})",
            short_sha(commit)
        ),
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
        EventKind::TargetDelegated { target, adapter } => {
            format!("  delegated to CI: {target} ({adapter})")
        }
        EventKind::TargetVerified { target, outcome } => {
            format!("  verified: {target} ({})", outcome.as_str())
        }
        EventKind::TagCreatedLocal { tag } => format!("  tag created: {tag}"),
        EventKind::TagPushedRemote { tag } => format!("  tag pushed: {tag}"),
        EventKind::GithubReleaseCreated { tag, url } => match url {
            Some(u) => format!("  release: {tag} ({u})"),
            None => format!("  release: {tag}"),
        },
        EventKind::GithubReleaseDelegated { tag, delegated_to } => {
            format!("  release delegated to CI: {tag} ({delegated_to})")
        }
        EventKind::RunAbandoned { reason } => format!("run abandoned: {reason}"),
    }
}

/// Text summary printed after a successful cut (json mode's summary is the stream).
fn render_cut_success(run_id: &str, plan: &ReleasePlan) -> Result<(), CliError> {
    crate::output::stdoutln!()?;
    crate::output::stdoutln!("release complete — run {run_id}")?;
    crate::output::stdoutln!("version: {}", plan.version)?;
    crate::output::stdoutln!("tag:     v{}", plan.version)?;
    // Publish-none must not read as a successful publish: a zero-target plan is a
    // TAG-ONLY cut (no registry publish, no GitHub Release), so say that rather than
    // reporting "published 0 target(s)" — which invites the reader to assume a
    // publish happened and merely counted wrong.
    if plan.targets.is_empty() {
        crate::output::stdoutln!(
            "published nothing — tag-only cut (the contract declares no publish targets)"
        )?;
    } else {
        crate::output::stdoutln!("published {} target(s)", plan.targets.len())?;
    }
    Ok(())
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

/// Parse the opt-in `--bump <level>` value into a
/// [`BumpLevel`](ossctl_core::protocol::plan::BumpLevel), or `None` when the
/// flag is absent (the default publish-the-tree-version path).
///
/// Strict enum validation (AI-first CLI canon): an unrecognized value is a hard,
/// informative error naming the valid levels — never a silent fallback.
fn parse_bump_level(
    raw: Option<&str>,
) -> Result<Option<ossctl_core::protocol::plan::BumpLevel>, CliError> {
    use ossctl_core::protocol::plan::BumpLevel;
    let Some(raw) = raw else {
        return Ok(None);
    };
    match BumpLevel::parse(raw) {
        Some(level) => Ok(Some(level)),
        None => Err(CliError::user(
            "invalid_bump",
            format!(
                "--bump must be one of {} (got '{raw}')",
                BumpLevel::VALID.join(", ")
            ),
        )
        .with_invalid_value(raw.to_string())
        .with_expected(serde_json::json!({ "one_of": BumpLevel::VALID }))),
    }
}

/// Map a strict-semver [`BumpError`](ossctl_core::release::bump::BumpError) from the
/// core bump constructor to the §10 error envelope.
///
/// The engine derives the number (there is no hand-typed literal version); a
/// non-semver manifest version fails **closed** rather than producing a wrong release
/// version.
fn bump_error_to_cli(
    level: ossctl_core::protocol::plan::BumpLevel,
    err: ossctl_core::release::bump::BumpError,
) -> CliError {
    CliError::user(
        "unbumpable_version",
        format!(
            "cannot compute a --bump {} from the current manifest version '{}': {}",
            level.as_str(),
            err.version,
            err.reason
        ),
    )
    .with_invalid_value(err.version)
}

/// The refusal returned when a cut targets an engine-owned bump plan: the plan side
/// (compute + seal, `release-rust-workspace-multicrate` facet 2/3) has landed, but the
/// cut-time execution of the bump phase is a follow-up validated by a real cut. Exit 1
/// — fail closed rather than build/publish the un-bumped manifest version. Takes the
/// [`BumpPlan`](ossctl_core::protocol::plan::BumpPlan) directly (the caller guards
/// `bump.is_some()`), so the message never
/// fabricates an empty `→` from an absent bump.
fn resolve_repo_root(flag: Option<&PathBuf>) -> Result<PathBuf, CliError> {
    match flag {
        Some(p) => Ok(p.clone()),
        None => std::env::current_dir()
            .map_err(|e| CliError::system("io_error", format!("cannot resolve cwd: {e}"))),
    }
}

/// Map an absent contract to the caller-actionable not-found class; failures
/// reading an existing contract remain operational errors.
fn load_error_to_cli(e: LoadError) -> CliError {
    let message = e.to_string();
    match e {
        LoadError::NotFound(_) => CliError::user("contract_not_found", message),
        LoadError::Io(..) => CliError::system("io_error", message),
        LoadError::Utf8(_) => CliError::system("invalid_encoding", message),
    }
}

/// An invalid contract is a caller-fixable (exit-1) error carrying every problem
/// — the plan cannot seal against a config that would not normalize.
/// Refuse a multi-distribution monorepo (`distributions.len() > 1`) in the
/// release-engine path (plan / cut / resume).
///
/// The contract models several independently-distributed binaries, but the
/// release engine still cuts ONE binary distribution per run: the sealed plan
/// carries a single `homebrew_tap`, so a second distribution's tap would be
/// silently dropped at the irreversible publish. Fail loud until per-distribution
/// release lands, mirroring `dist generate`'s `multiple_distributions`. The
/// single-distribution common case (`len <= 1`, incl. ossctl itself) is
/// unaffected.
/// Refuse a cut whose repository distribution surface is absent from its contract.
/// This is deliberately before journal creation: proceeding could create an empty
/// GitHub Release that prevents cargo-dist from publishing its artifacts.
fn ensure_declared_distribution(
    contract: &Contract,
    facts: &ossctl_core::protocol::facts::Facts,
) -> Result<(), CliError> {
    let findings = find_undeclared_distribution(
        &contract.targets,
        &facts.distribution_surface,
        contract.distributions.iter().any(|distribution| {
            distribution.homebrew_tap.is_some()
                && !distribution
                    .installers
                    .contains(&ossctl_core::contract::schema::Installer::Homebrew)
        }),
    );
    let Some(finding) = findings.first() else {
        return Ok(());
    };
    let message = match finding {
        UndeclaredDistribution::GhReleases { evidence } => format!(
            "{} detected, but OSS-RELEASE.md targets: has no registry: gh-releases target. Add {{ecosystem, package, registry: gh-releases, adapter: cargo-dist}} to OSS-RELEASE.md's targets: and re-plan; otherwise the tag phase would collide with cargo-dist and drop its binaries and Homebrew publish",
            evidence.join(", ")
        ),
        UndeclaredDistribution::Homebrew => "distribution.homebrew_tap is set, but OSS-RELEASE.md targets: has no registry: homebrew target. Add {ecosystem, package, registry: homebrew, adapter: homebrew-tap} to OSS-RELEASE.md's targets: and re-plan; otherwise the tap leg would be silently skipped".to_string(),
    };
    Err(CliError::user("undeclared_distribution", message))
}

/// Refuse a cut whose engine-published crates.io target depends on a CI-delegated
/// crate in the same workspace (`release-ci-publish-mode`). The engine publishes in
/// publish-all, before the tag push that triggers CI, so the dependency can never be
/// index-visible in time; the cut would burn the cargo adapter's index-wait and fail.
fn ensure_no_delegated_dependency_conflict(
    plan: &ossctl_core::protocol::plan::ReleasePlan,
    facts: &ossctl_core::protocol::facts::Facts,
) -> Result<(), CliError> {
    let conflicts = ossctl_core::release::plan::delegated_dependency_conflicts(plan, facts);
    let messages = ossctl_core::release::plan::delegated_dependency_messages(&conflicts);
    match messages.first() {
        None => Ok(()),
        Some(message) => Err(CliError::user(
            "delegated_dependency_conflict",
            message.clone(),
        )),
    }
}

fn ensure_single_distribution(contract: &Contract) -> Result<(), CliError> {
    if contract.distributions.len() > 1 {
        let packages: Vec<&str> = contract
            .distributions
            .iter()
            .map(|d| d.package.as_deref().unwrap_or("<unnamed>"))
            .collect();
        return Err(CliError::user(
            "multiple_distributions",
            format!(
                "the release engine cuts one binary distribution per run, but the contract \
                 declares {} ({}) — a multi-distribution monorepo is not yet cut end-to-end (its \
                 per-package homebrew taps would be dropped at publish). Per-distribution release \
                 is a follow-up",
                contract.distributions.len(),
                packages.join(", "),
            ),
        ));
    }
    Ok(())
}

fn invalid_contract_error(normalized: &Normalized) -> CliError {
    let problems = &normalized.problems.errors;
    let message = format!(
        "{} would not normalize: {} problem(s) — fix the contract before planning",
        contract::CONTRACT_FILENAME,
        problems.len()
    );
    CliError::user("invalid_contract", message).with_problems(problems.clone())
}

fn render_plan_text(plan: &ReleasePlan, warnings: &[String]) -> Result<(), CliError> {
    crate::output::stdoutln!("plan_id:    {}", plan.plan_id)?;
    crate::output::stdoutln!("head:       {}", plan.head_sha)?;
    crate::output::stdoutln!("version:    {}", plan.version)?;
    if let Some(bump) = &plan.bump {
        crate::output::stdoutln!(
            "bump:       {} ({} → {})",
            bump.level.as_str(),
            bump.from_version,
            bump.to_version
        )?;
        for r in &bump.pin_rewrites {
            crate::output::stdoutln!(
                "  pin:      {} depends on {} : {} → {}",
                r.in_package,
                r.dependency,
                r.from,
                r.to
            )?;
        }
        if bump.changelog_finalize {
            crate::output::stdoutln!("  changelog: finalize [Unreleased] → [{}]", bump.to_version)?;
        }
        if let Some(hook) = &bump.bump_hook {
            // Quoted (Debug) so a hook value carrying newlines/control chars cannot spoof
            // the surrounding plan output the approver reads.
            crate::output::stdoutln!("  bump_hook: {hook:?}")?;
        }
    }
    crate::output::stdoutln!("targets:    {}", plan.targets.len())?;
    for t in &plan.targets {
        crate::output::stdoutln!(
            "  {:<8} {:<12} {:<20} (package: {})",
            t.ecosystem.as_str(),
            t.registry.as_str(),
            t.adapter.as_str(),
            t.package.as_deref().unwrap_or("<inferred at cut>"),
        )?;
    }
    let phases = plan
        .phases
        .iter()
        .map(|p| p.as_str())
        .collect::<Vec<_>>()
        .join(" → ");
    crate::output::stdoutln!("phases:     {phases}")?;
    for w in warnings {
        crate::output::stdoutln!("warning:    {w}")?;
    }
    crate::output::stdoutln!()?;
    crate::output::stdoutln!("To execute this exact plan (refuses if the repo drifts):")?;
    crate::output::stdoutln!("  ossctl release cut --plan {}", plan.plan_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;
    use ossctl_core::contract::schema::{
        Adapter, Changelog, ChangelogMode, ChangelogSource, ContributionProvenance, DependencyBot,
        Distribution, DistributionAdapter, DocsSite, Ecosystem, Maturity, ProvenanceLevel,
        Registry, Release, ReleaseLayout, ReleaseModel, Target, VersioningBase,
    };
    use ossctl_core::protocol::facts::{DistributionSurface, Facts, MaturitySignals};
    use ossctl_core::protocol::journal::EventKind;

    /// A minimal approved `Contract` carrying the given distributions — enough to
    /// exercise the release-engine multi-distribution guard.
    const COMMIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const COMMIT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn downstream_tree_skips_provenance_check_silently() {
        assert_eq!(
            compiled_provenance_warning(false, COMMIT_A, COMMIT_B, false).unwrap(),
            None
        );
    }

    #[test]
    fn matching_self_cut_binary_provenance_passes_silently() {
        assert_eq!(
            compiled_provenance_warning(true, COMMIT_A, COMMIT_A, false).unwrap(),
            None
        );
    }

    #[test]
    fn mismatched_self_cut_binary_provenance_refuses_without_escape_hatch() {
        let error = compiled_provenance_warning(true, COMMIT_A, COMMIT_B, false).unwrap_err();
        assert_eq!(error.code, "stale_binary");
        assert!(error.message.contains(COMMIT_A));
        assert!(error.message.contains(COMMIT_B));
        assert!(error.message.contains("this ossctl checkout"));
    }

    #[test]
    fn stale_self_cut_binary_escape_hatch_emits_a_loud_warning() {
        let warning = compiled_provenance_warning(true, COMMIT_A, COMMIT_B, true)
            .unwrap()
            .expect("escape hatch must remain visible");
        assert!(warning.contains("STALE BINARY"));
        assert!(warning.contains("--allow-stale-binary"));
    }

    #[test]
    fn unknown_provenance_refuses_for_a_self_cut() {
        let error = compiled_provenance_warning(true, "unknown", COMMIT_A, false).unwrap_err();
        assert_eq!(error.code, "unverifiable_binary_provenance");
        assert!(error.message.contains("CANNOT VERIFY BINARY"));
        assert!(error.message.contains("cargo build --release -p ossctl"));
    }

    #[test]
    fn canonical_origin_identifies_only_ossctl_source_trees() {
        assert!(is_ossctl_source_tree(
            "git@github.com:jarimustonen/ossctl.git"
        ));
        assert!(is_ossctl_source_tree(
            "https://github.com/jarimustonen/ossctl"
        ));
        assert!(!is_ossctl_source_tree(
            "https://github.com/someone/ossctl.git"
        ));
        assert!(!is_ossctl_source_tree(
            "https://github.com/jarimustonen/other.git"
        ));
    }

    fn contract_with_distributions(dists: Vec<Distribution>) -> Contract {
        Contract {
            schema_version: 2,
            status: Status::Approved,
            maturity: Maturity::Production,
            ecosystems: vec![Ecosystem::Rust],
            targets: vec![],
            distributions: dists,
            versioning: VersioningBase::Semver,
            versioning_pattern: None,
            changelog: Changelog {
                mode: ChangelogMode::Curated,
                source: ChangelogSource::Manual,
                fragment_dir: "changelog/fragments".to_string(),
            },
            conventional_commits: false,
            release: Release {
                model: ReleaseModel::Gated,
                layout: ReleaseLayout::Single,
                bump_hook: None,
            },
            contribution_provenance: ContributionProvenance::None,
            provenance_level: ProvenanceLevel::None,
            dependency_bot: DependencyBot::None,
            health_badges: vec![],
            license: "MIT".to_string(),
            docs_site: DocsSite::None,
            extra_fields: serde_json::Map::new(),
            warnings: vec![],
        }
    }

    fn dist(package: &str) -> Distribution {
        Distribution {
            package: Some(package.to_string()),
            adapter: DistributionAdapter::CargoDist,
            gh_releases: true,
            installers: vec![],
            homebrew_tap: None,
            platforms: vec!["x86_64-unknown-linux-musl".to_string()],
            extra_fields: serde_json::Map::new(),
        }
    }

    /// The single-distribution common case (zero or one) passes the guard — the
    /// engine cuts it exactly as before.
    #[test]
    fn ensure_single_distribution_allows_zero_or_one() {
        assert!(ensure_single_distribution(&contract_with_distributions(vec![])).is_ok());
        assert!(
            ensure_single_distribution(&contract_with_distributions(vec![dist("solo")])).is_ok()
        );
    }

    /// A monorepo (≥2 distributions) is refused loudly before any irreversible
    /// publish — with the `multiple_distributions` code naming every package, so a
    /// second distribution's homebrew tap is never silently dropped.
    #[test]
    fn ensure_single_distribution_rejects_a_monorepo() {
        let c = contract_with_distributions(vec![dist("alpha"), dist("beta")]);
        let err = ensure_single_distribution(&c).unwrap_err();
        assert_eq!(err.code, "multiple_distributions");
        assert!(err.message.contains("alpha") && err.message.contains("beta"));
    }

    fn facts_with_surface(surface: DistributionSurface) -> Facts {
        Facts {
            repo_root: "/repo".to_string(),
            is_git: true,
            has_commits: true,
            ecosystems: vec![Ecosystem::Rust],
            packages: vec![],
            committers_total: 1,
            committers_recent_year: 1,
            tags: vec![],
            has_semver_tag: false,
            has_ge_1_0_release: false,
            has_ci: true,
            dependency_bot: None,
            has_issues_dir: false,
            readme_self_label: None,
            description: None,
            maturity_signals: MaturitySignals {
                production: false,
                spike: false,
            },
            inferred_maturity: Maturity::Mvp,
            distribution_surface: surface,
            rust_workspace: None,
        }
    }

    #[test]
    fn undeclared_distribution_preflight_refuses_both_missing_targets() {
        let mut contract = contract_with_distributions(vec![Distribution {
            homebrew_tap: Some("owner/tap".to_string()),
            ..dist("solo")
        }]);
        let facts = facts_with_surface(DistributionSurface {
            has_cargo_dist: true,
            cargo_dist_evidence: vec!["dist-workspace.toml".to_string()],
            tag_triggered_workflows: vec!["release.yml".to_string()],
        });
        let err = ensure_declared_distribution(&contract, &facts).unwrap_err();
        assert_eq!(err.code, "undeclared_distribution");
        assert!(err.message.contains("dist-workspace.toml"));
        assert!(err.message.contains("gh-releases"));

        contract.targets.push(Target {
            ecosystem: Ecosystem::Rust,
            package: Some("solo".to_string()),
            registry: Registry::GhReleases,
            adapter: Adapter::CargoDist,
        });
        let err = ensure_declared_distribution(&contract, &facts).unwrap_err();
        assert_eq!(err.code, "undeclared_distribution");
        assert!(err.message.contains("homebrew"));

        contract.targets.push(Target {
            ecosystem: Ecosystem::Rust,
            package: Some("solo".to_string()),
            registry: Registry::Homebrew,
            adapter: Adapter::HomebrewTap,
        });
        assert!(ensure_declared_distribution(&contract, &facts).is_ok());
    }

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
            head_sha: None,
            bump: None,
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

    /// A minimal `Parser` wrapper so `AbandonArgs` can be exercised through clap's
    /// argument parsing in isolation (mirrors how the real CLI embeds it under
    /// `release abandon`).
    #[derive(clap::Parser, Debug)]
    struct AbandonHarness {
        #[command(flatten)]
        args: AbandonArgs,
    }

    /// Clap harnesses for `plan` / `cut` argument parsing, to assert the `--version`
    /// input is gone (`release-drop-version-flag`).
    #[derive(clap::Parser, Debug)]
    struct PlanHarness {
        #[command(flatten)]
        args: PlanArgs,
    }
    #[derive(clap::Parser, Debug)]
    struct CutHarness {
        #[command(flatten)]
        args: CutArgs,
    }

    /// `release plan --version X.Y.Z` is a HARD ERROR, not a silently-ignored flag:
    /// the release version comes solely from the workspace manifest, so a stray
    /// `--version` fails loudly at the clap boundary rather than misleading a caller
    /// into thinking it set the version (`release-drop-version-flag`).
    #[test]
    fn plan_rejects_the_removed_version_flag() {
        assert!(
            PlanHarness::try_parse_from(["plan"]).is_ok(),
            "plan still parses with no --version"
        );
        let err = PlanHarness::try_parse_from(["plan", "--version", "0.3.0"])
            .expect_err("--version must be rejected");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "--version must be an unexpected-argument error, not ignored or some other clap error"
        );
    }

    /// `release cut --plan <id> --version X.Y.Z` likewise rejects the removed flag,
    /// while `--plan` alone still parses.
    #[test]
    fn cut_rejects_the_removed_version_flag() {
        assert!(
            CutHarness::try_parse_from(["cut", "--plan", "abc"]).is_ok(),
            "cut still parses with just --plan"
        );
        let err = CutHarness::try_parse_from(["cut", "--plan", "abc", "--version", "0.3.0"])
            .expect_err("--version must be rejected");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "--version must be an unexpected-argument error, not ignored or some other clap error"
        );
    }

    /// The opt-in `--bump <level>` parses on both `plan` and `cut` (symmetry: cut
    /// recomputes the same version to drift-check the sealed bump plan).
    #[test]
    fn plan_and_cut_accept_the_bump_flag() {
        let p = PlanHarness::try_parse_from(["plan", "--bump", "minor"]).unwrap();
        assert_eq!(p.args.bump.as_deref(), Some("minor"));
        let c = CutHarness::try_parse_from(["cut", "--plan", "abc", "--bump", "major"]).unwrap();
        assert_eq!(c.args.bump.as_deref(), Some("major"));
        // Omitting it is the default (no bump).
        assert!(PlanHarness::try_parse_from(["plan"])
            .unwrap()
            .args
            .bump
            .is_none());
    }

    /// `parse_bump_level` accepts exactly major/minor/patch, maps absent → `None`, and
    /// rejects any other value with an informative `invalid_bump` error naming the
    /// valid set (AI-first strict validation).
    #[test]
    fn parse_bump_level_validates_the_enum() {
        use ossctl_core::protocol::plan::BumpLevel;
        assert_eq!(parse_bump_level(None).unwrap(), None);
        assert_eq!(
            parse_bump_level(Some("major")).unwrap(),
            Some(BumpLevel::Major)
        );
        assert_eq!(
            parse_bump_level(Some("minor")).unwrap(),
            Some(BumpLevel::Minor)
        );
        assert_eq!(
            parse_bump_level(Some("patch")).unwrap(),
            Some(BumpLevel::Patch)
        );

        let err = parse_bump_level(Some("bugfix")).expect_err("a bad level must be rejected");
        assert_eq!(err.code, "invalid_bump");
        assert_eq!(err.invalid_value.as_deref(), Some("bugfix"));
        // The message names the valid levels.
        assert!(err.message.contains("major, minor, patch"));
    }

    /// `bump_error_to_cli` maps a core `BumpError` (non-semver manifest version) to the
    /// fail-closed `unbumpable_version` envelope, echoing the offending version.
    #[test]
    fn bump_error_to_cli_fails_closed_on_a_non_semver_version() {
        use ossctl_core::protocol::plan::BumpLevel;
        let err = ossctl_core::release::bump::bump_version(BumpLevel::Patch, "not-semver")
            .expect_err("non-semver must fail closed");
        let cli = bump_error_to_cli(BumpLevel::Patch, err);
        assert_eq!(cli.code, "unbumpable_version");
        assert_eq!(cli.invalid_value.as_deref(), Some("not-semver"));
    }

    /// A `--reason` value beginning with `--` must be taken literally, not parsed
    /// as an unknown flag (`allow_hyphen_values`). Regression for
    /// `release-abandon-reason-leading-dashes`.
    #[test]
    fn abandon_reason_accepts_leading_dashes() {
        let parsed = AbandonHarness::try_parse_from([
            "abandon",
            "RUN01",
            "--reason",
            "--no-verify insufficient; cargo package still resolves",
        ])
        .expect("a leading-dash --reason value must parse literally");
        assert_eq!(parsed.args.run_id, "RUN01");
        assert_eq!(
            parsed.args.reason.as_deref(),
            Some("--no-verify insufficient; cargo package still resolves"),
        );
    }

    /// The `--reason=<value>` binding form also carries a leading-dash value.
    #[test]
    fn abandon_reason_equals_form_accepts_leading_dashes() {
        let parsed = AbandonHarness::try_parse_from(["abandon", "RUN01", "--reason=--foo bar"])
            .expect("--reason=<value> must accept a leading-dash value");
        assert_eq!(parsed.args.reason.as_deref(), Some("--foo bar"));
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
