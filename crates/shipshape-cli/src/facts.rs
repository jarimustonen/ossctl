//! `shipshape facts` handler.
//!
//! Runs the deterministic detector (`shipshape_core::facts`) over the repo through
//! the real [`RealFs`] / [`RealGitRepo`] ports and emits the schema-versioned
//! facts report — the same facts `/shipshape-init` and the readiness `audit` both
//! read, so they never disagree on maturity or the gated core (ADR-0001 §3).

use std::path::PathBuf;

use clap::Args;

use shipshape_core::protocol::facts::FactsReport;

use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::{RealFs, RealGitRepo};

/// Arguments for `shipshape facts`.
#[derive(Args, Debug)]
pub struct FactsArgs {
    /// Repository root to inspect (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
}

/// `shipshape facts` — detect deterministic repo facts.
pub fn run(args: &FactsArgs, format: OutputFormat) -> Result<(), CliError> {
    let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
    if !repo_root.is_dir() {
        return Err(CliError::user(
            "invalid_repo_root",
            format!("repo_root '{}' is not a directory", repo_root.display()),
        )
        .with_invalid_value(repo_root.display().to_string()));
    }
    // Canonicalize so `repo_root` in the report is absolute + symlink-resolved
    // (mirrors the Python detector's `os.path.realpath`). A failure here (a race
    // after the is_dir check, a permission error) is a system error rather than
    // a silent fall back to a possibly-relative path — the emitted `repo_root`
    // is part of the contract.
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
    let report = shipshape_core::facts::gather_report(&root, &RealFs, &git);

    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &[])?,
        OutputFormat::Text => render_facts_text(&report)?,
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

fn render_facts_text(report: &FactsReport) -> Result<(), CliError> {
    let f = &report.facts;
    let ecos = if f.ecosystems.is_empty() {
        "[]".to_string()
    } else {
        f.ecosystems
            .iter()
            .map(|e| e.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    crate::output::stdoutln!("repo_root:         {}", f.repo_root)?;
    crate::output::stdoutln!("is_git:            {}", f.is_git)?;
    crate::output::stdoutln!("has_commits:       {}", f.has_commits)?;
    crate::output::stdoutln!("ecosystems:        {ecos}")?;
    crate::output::stdoutln!("packages:          {}", f.packages.len())?;
    crate::output::stdoutln!(
        "cargo_publish:     {} manifest(s)",
        report.cargo_publish.len()
    )?;
    for evidence in &report.cargo_publish {
        crate::output::stdoutln!(
            "  {} (package={}): {}",
            evidence.manifest,
            evidence.package.as_deref().unwrap_or("unknown"),
            evidence.policy.as_str()
        )?;
    }
    crate::output::stdoutln!("has_ci:            {}", f.has_ci)?;
    crate::output::stdoutln!("tags:              {}", f.tags.len())?;
    crate::output::stdoutln!("has_semver_tag:    {}", f.has_semver_tag)?;
    crate::output::stdoutln!("has_ge_1_0:        {}", f.has_ge_1_0_release)?;
    crate::output::stdoutln!(
        "committers:        {} total, {} recent-year",
        f.committers_total,
        f.committers_recent_year
    )?;
    if let Some(bot) = &f.dependency_bot {
        crate::output::stdoutln!("dependency_bot:    {bot}")?;
    }
    if let Some(label) = &f.readme_self_label {
        crate::output::stdoutln!("readme_self_label: {label}")?;
    }
    if let Some(desc) = &f.description {
        crate::output::stdoutln!("description:       {desc}")?;
    }
    // Surface the raw signals so a human can see what drives the maturity call.
    crate::output::stdoutln!(
        "maturity_signals:  production={}, spike={}",
        f.maturity_signals.production,
        f.maturity_signals.spike
    )?;
    crate::output::stdoutln!("inferred_maturity: {}", f.inferred_maturity.as_str())?;
    Ok(())
}
