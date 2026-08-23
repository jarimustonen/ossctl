//! `shipshape audit` handler.
//!
//! Runs the read-only readiness scorer (`shipshape_core::audit`) over the repo and
//! emits the schema-versioned gap-report. The scorer is a pure function of
//! `(repo tree, contract, facts)`: this handler obtains the contract by running
//! the same normalizer behind `contract show` and the facts by running the same
//! detector behind `facts`, then hands both to the engine — so the audit,
//! `/shipshape-init`, and every other member agree on maturity and the gated core
//! (ADR-0001 §3). The GitHub community-standards lookup goes through the real
//! [`RealCommandRunner`] port; a lookup failure degrades to `unknown`, never
//! `false`. Never writes the repo.

use std::path::PathBuf;

use clap::Args;

use shipshape_core::contract::{self, LoadError, Normalized};
use shipshape_core::protocol::audit::AuditReport;

use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::{RealCommandRunner, RealFs, RealGitRepo};

/// Arguments for `shipshape audit`.
#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Repository root to score (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
}

/// `shipshape audit` — score release readiness into a gap report (read-only).
pub fn run(args: &AuditArgs, format: OutputFormat) -> Result<(), CliError> {
    let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
    if !repo_root.is_dir() {
        return Err(CliError::user(
            "invalid_repo_root",
            format!("repo_root '{}' is not a directory", repo_root.display()),
        )
        .with_invalid_value(repo_root.display().to_string()));
    }
    // Canonicalize so the emitted `repo_root` is absolute + symlink-resolved,
    // matching the facts detector's contract.
    let root = std::fs::canonicalize(&repo_root).map_err(|e| {
        CliError::system(
            "io_error",
            format!(
                "cannot canonicalize repo_root '{}': {e}",
                repo_root.display()
            ),
        )
    })?;

    // The audit reads an already-normalized contract; a missing or invalid
    // OSS-RELEASE.md is the same failure `contract show` reports (the
    // /shipshape-readiness skill gates on `contract show` before calling audit).
    let normalized = contract::normalize(&root, &RealFs).map_err(load_error_to_cli)?;
    if !normalized.is_valid() {
        return Err(invalid_contract_error(&normalized));
    }

    let git = RealGitRepo::new(&root);
    let facts = shipshape_core::facts::gather(&root, &RealFs, &git);

    let report = shipshape_core::audit::audit(
        &root,
        &normalized.contract,
        &facts,
        &RealFs,
        &RealCommandRunner,
    );

    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &[])?,
        OutputFormat::Text => render_audit_text(&report)?,
    }
    Ok(())
}

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
/// — the audit cannot score against a config that would not normalize.
fn invalid_contract_error(normalized: &Normalized) -> CliError {
    let problems = &normalized.problems.errors;
    let message = format!(
        "{} would not normalize: {} problem(s) — fix the contract before auditing",
        contract::CONTRACT_FILENAME,
        problems.len()
    );
    CliError::user("invalid_contract", message).with_problems(problems.clone())
}

fn render_audit_text(report: &AuditReport) -> Result<(), CliError> {
    let core = report.core_complete.as_str();
    crate::output::stdoutln!("repo_root:      {}", report.repo_root)?;
    crate::output::stdoutln!("maturity:       {}", report.maturity.as_str())?;
    crate::output::stdoutln!("gated core:     {core}")?;
    crate::output::stdoutln!("gaps:           {}", report.gaps.len())?;
    for g in &report.gaps {
        crate::output::stdoutln!(
            "  [{:>11}] {:<24} ({}, {}, {}) — {}",
            g.severity.as_str(),
            g.id,
            g.category.as_str(),
            g.member,
            g.status.as_str(),
            g.detail
        )?;
    }
    let cp = &report.community_profile;
    if cp.checked {
        crate::output::stdoutln!("community:      GitHub community-profile checked")?;
    } else {
        crate::output::stdoutln!(
            "community:      not checked ({})",
            cp.unavailable_reason.as_deref().unwrap_or("unknown")
        )?;
    }
    Ok(())
}
