//! `ossctl release …` handlers.
//!
//! `release plan` is implemented (the sealed content-addressed approval seam,
//! ADR-0002 §3); the remaining verbs (`cut`/`resume`/`verify`/`show`/`list`/
//! `abandon`) live in `ossctl-core::release` and land with their sibling units,
//! returning a clean `not_implemented` envelope until then. The argument shapes
//! are real so the surface and `--help` are accurate.

use std::path::PathBuf;

use clap::Args;

use ossctl_core::contract::{self, LoadError, Normalized};
use ossctl_core::ports::GitRepo;
use ossctl_core::protocol::journal::RunStatus;
use ossctl_core::protocol::plan::ReleasePlan;
use ossctl_core::protocol::reconcile::ReconcileReport;
use ossctl_core::release::adapters::EffectCtx;
use ossctl_core::release::journal::{self, JournalPaths};

use crate::cli::ReleaseAction;
use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::{
    ReadOnlyJournalStore, RealClock, RealCommandRunner, RealFs, RealGitRepo, RealRegistryQuery,
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
#[derive(Args, Debug)]
pub struct CutArgs {
    /// The sealed plan id to execute (from `release plan`).
    #[arg(long, value_name = "PLAN_ID")]
    pub plan: String,
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
        ReleaseAction::Cut(_) => Err(CliError::not_implemented("release cut")),
        ReleaseAction::Resume(_) => Err(CliError::not_implemented("release resume")),
        ReleaseAction::Verify(args) => verify(&args, format),
        ReleaseAction::Show(_) => Err(CliError::not_implemented("release show")),
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
            "target '{}' has no resolved package name (ambiguous or undetected) — it will be \
             inferred at cut time; pin an explicit 'package' in the contract to seal it",
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
    println!("  ossctl release cut --plan {}", plan.plan_id);
}
