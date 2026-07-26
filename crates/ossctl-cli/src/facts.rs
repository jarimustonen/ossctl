//! `ossctl facts` handler.
//!
//! Runs the deterministic detector (`ossctl_core::facts`) over the repo through
//! the real [`RealFs`] / [`RealGitRepo`] ports and emits the schema-versioned
//! facts report — the same facts `/oss-init` and the readiness `audit` both
//! read, so they never disagree on maturity or the gated core (ADR-0001 §3).

use std::path::PathBuf;

use clap::Args;

use ossctl_core::protocol::facts::Facts;

use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::{RealFs, RealGitRepo};

/// Arguments for `ossctl facts`.
#[derive(Args, Debug)]
pub struct FactsArgs {
    /// Repository root to inspect (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
}

/// `ossctl facts` — detect deterministic repo facts.
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
    let facts = ossctl_core::facts::gather(&root, &RealFs, &git);

    match format {
        OutputFormat::Json => crate::output::emit_json(&facts, &[])?,
        OutputFormat::Text => render_facts_text(&facts),
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

fn render_facts_text(f: &Facts) {
    let ecos = if f.ecosystems.is_empty() {
        "[]".to_string()
    } else {
        f.ecosystems
            .iter()
            .map(|e| e.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!("repo_root:         {}", f.repo_root);
    println!("is_git:            {}", f.is_git);
    println!("has_commits:       {}", f.has_commits);
    println!("ecosystems:        {ecos}");
    println!("packages:          {}", f.packages.len());
    println!("has_ci:            {}", f.has_ci);
    println!("tags:              {}", f.tags.len());
    println!("has_semver_tag:    {}", f.has_semver_tag);
    println!("has_ge_1_0:        {}", f.has_ge_1_0_release);
    println!(
        "committers:        {} total, {} recent-year",
        f.committers_total, f.committers_recent_year
    );
    if let Some(bot) = &f.dependency_bot {
        println!("dependency_bot:    {bot}");
    }
    if let Some(label) = &f.readme_self_label {
        println!("readme_self_label: {label}");
    }
    if let Some(desc) = &f.description {
        println!("description:       {desc}");
    }
    // Surface the raw signals so a human can see what drives the maturity call.
    println!(
        "maturity_signals:  production={}, spike={}",
        f.maturity_signals.production, f.maturity_signals.spike
    );
    println!("inferred_maturity: {}", f.inferred_maturity.as_str());
}
