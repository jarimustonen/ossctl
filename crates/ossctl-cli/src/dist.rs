//! `ossctl dist generate` handler.
//!
//! Generates a downstream project's binary-release infrastructure from the
//! contract's `distribution` block (issue `release-engine-dist-config-generator`,
//! Track B toward an engine-driven release):
//!
//! 1. render `dist-workspace.toml` from `distribution.platforms` /
//!    `distribution.installers` through the deterministic
//!    [`ossctl_core::dist`] generator (cross-platform by default — macOS AND
//!    Linux — never a narrower set), and write it to the repo root; then
//! 2. invoke `dist generate` (the cargo-dist tool) to produce the tag-triggered
//!    `.github/workflows/release.yml` from that config. The workflow is **never**
//!    hand-authored or templated — cargo-dist is its sole author (ADR-0001 keeps
//!    the deterministic machinery in the binary; the workflow bytes come from the
//!    tool, not from ossctl).
//!
//! This is a dedicated `dist` noun rather than a `release …` verb: `release`'s
//! verbs are the journaled, drift-checked, resumable *run* states (ADR-0002 §4),
//! whereas this is one-time, idempotent scaffolding that must exist **before** a
//! tag is pushed. It is not part of the phase-barrier cut.

use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use ossctl_core::contract::schema::{DistributionAdapter, Status};
use ossctl_core::contract::{self, LoadError, Normalized};
use ossctl_core::ports::CommandRunner;

use crate::cli::DistAction;
use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::{RealCommandRunner, RealFs};

/// The cargo-dist config file this command writes at the repo root.
const DIST_CONFIG_FILENAME: &str = "dist-workspace.toml";
/// The tag-triggered workflow `dist generate` produces from the config.
const RELEASE_WORKFLOW_PATH: &str = ".github/workflows/release.yml";

/// Arguments for `ossctl dist generate`.
#[derive(Args, Debug)]
pub struct GenerateArgs {
    /// Repository root containing OSS-RELEASE.md (default: current directory).
    /// The generated `dist-workspace.toml` and workflow are written here.
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Require `status: approved` before generating (for a mutating orchestrator
    /// that refuses to scaffold from a draft contract). Off by default so infra
    /// can be scaffolded during setup, before the human flips approval.
    #[arg(long)]
    pub require_approved: bool,
    /// Overwrite an existing `dist-workspace.toml` (refused without this flag so a
    /// hand-tuned config is never clobbered).
    #[arg(long)]
    pub force: bool,
    /// Write only `dist-workspace.toml` and skip the `dist generate` invocation
    /// that produces the workflow (for environments without the cargo-dist tool).
    #[arg(long)]
    pub no_workflow: bool,
}

/// Dispatch a `dist` subcommand to its handler.
pub fn dispatch(action: DistAction, format: OutputFormat) -> Result<(), CliError> {
    match action {
        DistAction::Generate(args) => generate(&args, format, &RealCommandRunner),
    }
}

/// The `data` body of a successful `dist generate`.
#[derive(Debug, Serialize)]
struct DistReport {
    /// Repo-relative path of the written config (`dist-workspace.toml`).
    dist_config: &'static str,
    /// Repo-relative path of the generated workflow, or `null` when
    /// `--no-workflow` skipped it.
    workflow: Option<&'static str>,
    /// The pinned cargo-dist version written into the config.
    cargo_dist_version: &'static str,
    /// The `[dist] targets` set emitted (verbatim from `distribution.platforms`).
    targets: Vec<String>,
    /// The `[dist] installers` set emitted (shell ensured, homebrew excluded).
    installers: Vec<String>,
}

/// `ossctl dist generate` — generate the cargo-dist release infra from the
/// contract's `distribution` block. The [`CommandRunner`] is injected so the
/// `dist generate` invocation is testable with a recording fake.
pub fn generate(
    args: &GenerateArgs,
    format: OutputFormat,
    runner: &dyn CommandRunner,
) -> Result<(), CliError> {
    let root = resolve_and_canonicalize(args.repo_root.as_ref())?;

    // A dist config is generated from a normalized contract; a missing or invalid
    // OSS-RELEASE.md is the same failure `contract show` reports.
    let normalized = contract::normalize(&root, &RealFs).map_err(load_error_to_cli)?;
    if !normalized.is_valid() {
        return Err(invalid_contract_error(&normalized));
    }
    if args.require_approved && normalized.contract.status != Status::Approved {
        return Err(CliError::user(
            "not_approved",
            format!(
                "{} is `{}`, not `approved` — a mutating orchestrator refuses to scaffold from a \
                 draft (drop --require-approved to generate anyway)",
                contract::CONTRACT_FILENAME,
                normalized.contract.status.as_str()
            ),
        )
        .with_invalid_value(normalized.contract.status.as_str().to_string()));
    }

    // The contract must declare a distribution block — there is nothing to
    // generate for a registry-only repo.
    let distribution = normalized.contract.distribution.as_ref().ok_or_else(|| {
        CliError::user(
            "no_distribution",
            format!(
                "{} declares no `distribution` block — there is no binary-release infra to \
                 generate. Add a distribution block (adapter, platforms, installers) to the \
                 contract, or use a registry-only release",
                contract::CONTRACT_FILENAME,
            ),
        )
    })?;

    // This generator emits cargo-dist config only; a goreleaser/manual adapter is
    // a different toolchain the generator does not (yet) know how to scaffold.
    if distribution.adapter != DistributionAdapter::CargoDist {
        return Err(CliError::user(
            "unsupported_distribution_adapter",
            format!(
                "`dist generate` scaffolds the cargo-dist adapter, but the contract's \
                 distribution.adapter is `{}`. Only `cargo-dist` is supported; a \
                 goreleaser/manual scaffolder is a separate follow-up",
                distribution.adapter.as_str()
            ),
        )
        .with_invalid_value(distribution.adapter.as_str())
        .with_expected(serde_json::json!([DistributionAdapter::CargoDist.as_str()])));
    }

    let generated = ossctl_core::dist::generate(distribution);

    // Write the config first (the workflow generation reads it). Refuse to clobber
    // a hand-tuned config unless --force.
    let config_path = root.join(DIST_CONFIG_FILENAME);
    if config_path.exists() && !args.force {
        return Err(CliError::user(
            "dist_config_exists",
            format!(
                "{DIST_CONFIG_FILENAME} already exists at {} — pass --force to overwrite it \
                 (a hand-tuned config is not clobbered by default)",
                config_path.display()
            ),
        )
        .with_invalid_value(config_path.display().to_string()));
    }
    std::fs::write(&config_path, &generated.toml).map_err(|e| {
        CliError::system(
            "io_error",
            format!("cannot write {}: {e}", config_path.display()),
        )
    })?;

    // Then produce the workflow via the tool (never hand-authored YAML), unless
    // the caller opted out.
    let mut warnings = generated.warnings.clone();
    let workflow = if args.no_workflow {
        warnings.push(format!(
            "skipped `dist generate` (--no-workflow); {DIST_CONFIG_FILENAME} was written but \
             {RELEASE_WORKFLOW_PATH} was NOT regenerated — run `dist generate` to produce it"
        ));
        None
    } else {
        run_dist_generate(runner, &root)?;
        Some(RELEASE_WORKFLOW_PATH)
    };

    let report = DistReport {
        dist_config: DIST_CONFIG_FILENAME,
        workflow,
        cargo_dist_version: generated.cargo_dist_version,
        targets: generated.targets.clone(),
        installers: generated.installers.clone(),
    };
    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &warnings)?,
        OutputFormat::Text => render_text(&report, &warnings),
    }
    Ok(())
}

/// Invoke `dist generate` in `root` to (re)produce the tag-triggered workflow
/// from the just-written `dist-workspace.toml`.
///
/// The config is already on disk, so both failure modes name it: a missing
/// cargo-dist tool is a system error with an install pointer; a non-zero `dist`
/// exit surfaces its stderr. Either way the caller can re-run just the workflow
/// step once the tool is available (`dist generate`) — the config need not be
/// regenerated.
fn run_dist_generate(runner: &dyn CommandRunner, root: &Path) -> Result<(), CliError> {
    let output = runner.run("dist", &["generate"], root).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CliError::system(
                "dist_tool_missing",
                format!(
                    "wrote {DIST_CONFIG_FILENAME}, but the `dist` (cargo-dist) tool is not \
                     installed, so {RELEASE_WORKFLOW_PATH} was not generated. Install it \
                     (`cargo install cargo-dist` or the curl installer from \
                     https://opensource.axo.dev/cargo-dist/) and re-run `dist generate`, or pass \
                     --no-workflow to skip this step"
                ),
            )
        } else {
            CliError::system(
                "dist_generate_failed",
                format!("wrote {DIST_CONFIG_FILENAME}, but running `dist generate` failed: {e}"),
            )
        }
    })?;
    if output.status != Some(0) {
        let detail = if output.stderr.trim().is_empty() {
            output.stdout.trim().to_string()
        } else {
            output.stderr.trim().to_string()
        };
        return Err(CliError::system(
            "dist_generate_failed",
            format!(
                "wrote {DIST_CONFIG_FILENAME}, but `dist generate` exited with status {} — \
                 {RELEASE_WORKFLOW_PATH} may be incomplete. Details: {detail}",
                output
                    .status
                    .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            ),
        ));
    }
    Ok(())
}

fn render_text(report: &DistReport, warnings: &[String]) {
    println!("wrote:              {}", report.dist_config);
    match report.workflow {
        Some(w) => println!("generated:          {w}"),
        None => println!("generated:          (skipped — --no-workflow)"),
    }
    println!("cargo-dist-version: {}", report.cargo_dist_version);
    println!("installers:         {}", report.installers.join(", "));
    println!("targets:            {}", report.targets.len());
    for t in &report.targets {
        println!("  {t}");
    }
    for w in warnings {
        println!("warning:            {w}");
    }
}

/// Resolve `--repo-root` (default cwd) and canonicalize it, mirroring the other
/// contract-reading handlers so the emitted paths are absolute + symlink-resolved.
fn resolve_and_canonicalize(flag: Option<&PathBuf>) -> Result<PathBuf, CliError> {
    let repo_root = match flag {
        Some(p) => p.clone(),
        None => std::env::current_dir()
            .map_err(|e| CliError::system("io_error", format!("cannot resolve cwd: {e}")))?,
    };
    if !repo_root.is_dir() {
        return Err(CliError::user(
            "invalid_repo_root",
            format!("repo_root '{}' is not a directory", repo_root.display()),
        )
        .with_invalid_value(repo_root.display().to_string()));
    }
    std::fs::canonicalize(&repo_root).map_err(|e| {
        CliError::system(
            "io_error",
            format!(
                "cannot canonicalize repo_root '{}': {e}",
                repo_root.display()
            ),
        )
    })
}

/// A failed contract load is a system-level (exit-2) error — the generator could
/// not obtain the config it reads.
fn load_error_to_cli(e: LoadError) -> CliError {
    let code = match e {
        LoadError::NotFound(_) => "contract_not_found",
        LoadError::Io(..) => "io_error",
        LoadError::Utf8(_) => "invalid_encoding",
    };
    CliError::system(code, e.to_string())
}

/// An invalid contract is a caller-fixable (exit-1) error carrying every problem.
fn invalid_contract_error(normalized: &Normalized) -> CliError {
    let problems = &normalized.problems.errors;
    let message = format!(
        "{} would not normalize: {} problem(s) — fix the contract before generating dist config",
        contract::CONTRACT_FILENAME,
        problems.len()
    );
    CliError::user("invalid_contract", message).with_problems(problems.clone())
}

#[cfg(test)]
#[path = "dist_tests.rs"]
mod tests;
