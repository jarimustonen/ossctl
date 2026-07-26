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
use ossctl_core::protocol::plan::ReleasePlan;

use crate::cli::ReleaseAction;
use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::{RealFs, RealGitRepo};

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

/// A single positional `<run_id>`, shared by `resume` / `verify` / `show`.
#[derive(Args, Debug)]
pub struct RunIdArgs {
    /// The run id.
    #[arg(value_name = "RUN_ID")]
    pub run_id: String,
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
        ReleaseAction::Verify(_) => Err(CliError::not_implemented("release verify")),
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
    // A release must be sealed against a concrete commit; an unborn repo cannot
    // be planned (this is caller-fixable — commit first).
    let head_sha = git.head_commit().map_err(|_| {
        CliError::user(
            "no_head",
            "cannot plan a release: the repository has no commits (HEAD does not resolve)",
        )
    })?;

    let facts = ossctl_core::facts::gather(&root, &RealFs, &git);

    let plan = ossctl_core::release::plan::build(&normalized.contract, &facts, &head_sha, version);

    // Surface a non-blocking warning when the contract configures nothing to
    // publish — the plan would tag only.
    let mut warnings = normalized.problems.warnings.clone();
    if plan.targets.is_empty() {
        warnings.push(
            "the contract declares no publish targets — this plan would create the git tag only"
                .to_string(),
        );
    }

    match format {
        OutputFormat::Json => crate::output::emit_json(&plan, &warnings)?,
        OutputFormat::Text => render_plan_text(&plan, &warnings),
    }
    Ok(())
}

/// Validate the chosen version: non-empty and free of whitespace. Scheme
/// specificity (semver vs a calver pattern) belongs to the contract/skill, so
/// the plan treats it as an opaque, already-chosen identifier and only rejects
/// the shapes that could never be a version.
fn validate_version(version: &str) -> Result<&str, CliError> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return Err(
            CliError::user("invalid_version", "the release version must not be empty")
                .with_invalid_value(version.to_string()),
        );
    }
    if version.chars().any(char::is_whitespace) {
        return Err(CliError::user(
            "invalid_version",
            "the release version must not contain whitespace",
        )
        .with_invalid_value(version.to_string()));
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
