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

use std::io::Write as _;
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

    // The contract must declare exactly one distribution block. A registry-only
    // repo declares none (nothing to generate); a monorepo declares several, which
    // this single-`dist-workspace.toml` generator cannot yet scaffold (per-package
    // dist generation is a follow-up).
    let distribution = match normalized.contract.distributions.as_slice() {
        [d] => d,
        [] => {
            return Err(CliError::user(
                "no_distribution",
                format!(
                    "{} declares no `distribution` block — there is no binary-release infra to \
                     generate. Add a distribution block (adapter, platforms, installers) to the \
                     contract, or use a registry-only release",
                    contract::CONTRACT_FILENAME,
                ),
            ));
        }
        many => {
            let packages: Vec<&str> = many
                .iter()
                .map(|d| d.package.as_deref().unwrap_or("<unnamed>"))
                .collect();
            return Err(CliError::user(
                "multiple_distributions",
                format!(
                    "{} declares {} distributions ({}) — `dist generate` scaffolds a single \
                     {DIST_CONFIG_FILENAME} and cannot yet target one package of a monorepo \
                     (per-package dist generation is a follow-up)",
                    contract::CONTRACT_FILENAME,
                    many.len(),
                    packages.join(", "),
                ),
            ));
        }
    };

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

    // Write the config first (the workflow generation reads it), then produce the
    // workflow via the tool (never hand-authored YAML).
    let config_path = root.join(DIST_CONFIG_FILENAME);
    write_config(&config_path, &generated.toml, args.force)?;

    let mut warnings = generated.warnings.clone();
    let workflow = if args.no_workflow {
        warnings.push(format!(
            "skipped the workflow step (--no-workflow); {DIST_CONFIG_FILENAME} was written but \
             {RELEASE_WORKFLOW_PATH} was NOT regenerated — re-run `ossctl dist generate` (without \
             --no-workflow) to produce it"
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
        OutputFormat::Text => render_text(&report, &warnings)?,
    }
    Ok(())
}

/// Write `contents` to `path`, protecting an existing hand-tuned or generated
/// config and never leaving a torn file behind.
///
/// - **Absent** → created atomically (`create_new`, which also closes the
///   TOCTOU window a separate `exists()` check would open).
/// - **Present and byte-identical** → a no-op: a re-run after a partial failure
///   (config written, `dist generate` then failed) is idempotent and does NOT
///   require `--force`. This is the natural retry story for scaffolding.
/// - **Present and different** → refused without `--force`; with `--force`,
///   replaced via a same-directory temp file + atomic `rename`, so a crash or
///   disk-full mid-write can never truncate the previous config.
fn write_config(path: &Path, contents: &str, force: bool) -> Result<(), CliError> {
    let io_err = |e: std::io::Error| {
        CliError::system("io_error", format!("cannot write {}: {e}", path.display()))
    };
    match std::fs::read(path) {
        Ok(existing) if existing == contents.as_bytes() => Ok(()), // already current
        Ok(_) if !force => Err(CliError::user(
            "dist_config_exists",
            format!(
                "{DIST_CONFIG_FILENAME} already exists at {} with different content — pass --force \
                 to overwrite it (a hand-tuned config is not clobbered by default)",
                path.display()
            ),
        )
        .with_invalid_value(path.display().to_string())),
        Ok(_) => atomic_replace(path, contents).map_err(io_err),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Exclusive create: fails if the file appears between the read above
            // and here, so a concurrent writer is never silently clobbered.
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(io_err)?;
            f.write_all(contents.as_bytes()).map_err(io_err)
        }
        Err(e) => Err(io_err(e)),
    }
}

/// Replace `path`'s contents atomically: write a sibling temp file, then rename
/// it over the target (an atomic swap on the same filesystem).
fn atomic_replace(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Invoke `dist generate` in `root` to (re)produce the tag-triggered workflow
/// from the just-written `dist-workspace.toml`, then confirm the workflow landed.
///
/// The config is already on disk, so both failure modes name it: a missing
/// cargo-dist tool is a system error with an install pointer; a non-zero `dist`
/// exit surfaces its stderr. A zero exit that produced no workflow is also an
/// error rather than a falsely-successful report — the command's contract is that
/// the workflow exists when it returns `Ok`.
fn run_dist_generate(runner: &dyn CommandRunner, root: &Path) -> Result<(), CliError> {
    let output = runner.run("dist", &["generate"], root).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CliError::system(
                "dist_tool_missing",
                format!(
                    "wrote {DIST_CONFIG_FILENAME}, but the `dist` (cargo-dist) tool is not \
                     installed, so {RELEASE_WORKFLOW_PATH} was not generated. Install it \
                     (`cargo install cargo-dist --version {pinned} --locked`, or the curl \
                     installer from https://opensource.axo.dev/cargo-dist/) and re-run \
                     `ossctl dist generate`, or pass --no-workflow to skip this step",
                    pinned = ossctl_core::dist::PINNED_CARGO_DIST_VERSION,
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
        let detail = match (output.stderr.trim(), output.stdout.trim()) {
            ("", "") => "cargo-dist produced no diagnostic output".to_string(),
            ("", out) => out.to_string(),
            (err, "") => err.to_string(),
            (err, out) => format!("{err}\n{out}"),
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
    // A zero exit is not proof the workflow was produced — a different cargo-dist
    // build or config interpretation could exit clean without writing it. Confirm
    // the file exists so the success report is truthful.
    let workflow_path = root.join(RELEASE_WORKFLOW_PATH);
    if !workflow_path.is_file() {
        return Err(CliError::system(
            "dist_workflow_missing",
            format!(
                "`dist generate` exited 0 but did not produce {} — the installed cargo-dist may \
                 differ from the pinned {} in {DIST_CONFIG_FILENAME}",
                workflow_path.display(),
                ossctl_core::dist::PINNED_CARGO_DIST_VERSION,
            ),
        ));
    }
    Ok(())
}

fn render_text(report: &DistReport, warnings: &[String]) -> Result<(), CliError> {
    crate::output::stdoutln!("wrote:              {}", report.dist_config)?;
    match report.workflow {
        Some(w) => crate::output::stdoutln!("generated:          {w}")?,
        None => crate::output::stdoutln!("generated:          (skipped — --no-workflow)")?,
    }
    crate::output::stdoutln!("cargo-dist-version: {}", report.cargo_dist_version)?;
    crate::output::stdoutln!("installers:         {}", report.installers.join(", "))?;
    crate::output::stdoutln!("targets:            {}", report.targets.len())?;
    for t in &report.targets {
        crate::output::stdoutln!("  {t}")?;
    }
    for w in warnings {
        crate::output::stdoutln!("warning:            {w}")?;
    }
    Ok(())
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
