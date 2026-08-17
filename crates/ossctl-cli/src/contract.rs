//! `contract show` / `contract validate` handlers.
//!
//! `contract show` is THE normalizer / single reader of `OSS-RELEASE.md`: it
//! emits the canonical, defaulted, `targets`-expanded, schema-versioned document
//! (SCHEMA.md §4) under the CLI's `data` envelope. `contract validate` runs the
//! identical pipeline and emits only pass/fail — the §10 error envelope (with
//! the full problem list) on failure, no document body (ADR-0001 §1).
//!
//! Both resolve `OSS-RELEASE.md` under `--repo-root` (default: current
//! directory) and gate on `--require-approved` for mutating callers.

use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use ossctl_core::contract::{self, LoadError, Normalized};
use ossctl_core::protocol::contract::Contract;

use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::RealFs;

/// Shared repo-location + approval flags for the contract handlers.
#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Repository root containing OSS-RELEASE.md (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Require the contract to be in the approved state before emitting it.
    #[arg(long)]
    pub require_approved: bool,
}

/// Arguments for the check-only `contract validate` gate.
#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Repository root containing OSS-RELEASE.md (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Fail unless `status: approved` (mutating members pass this so a draft
    /// config can never authorize a mutation).
    #[arg(long)]
    pub require_approved: bool,
}

/// `contract show` — normalize and emit the canonical contract.
pub fn show(args: &ShowArgs, format: OutputFormat) -> Result<(), CliError> {
    let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
    let normalized = contract::normalize(&repo_root, &RealFs).map_err(load_error_to_cli)?;

    if !normalized.is_valid() {
        return Err(invalid_contract_error(&normalized));
    }
    if args.require_approved {
        require_approved(&normalized.contract)?;
    }

    let doc = &normalized.contract;
    match format {
        OutputFormat::Json => crate::output::emit_json(doc, &doc.warnings)?,
        OutputFormat::Text => render_show_text(doc)?,
    }
    Ok(())
}

/// `contract validate` — run the normalization pipeline and emit pass/fail.
pub fn validate(args: &ValidateArgs, format: OutputFormat) -> Result<(), CliError> {
    let repo_root = resolve_repo_root(args.repo_root.as_ref())?;
    let normalized = contract::normalize(&repo_root, &RealFs).map_err(load_error_to_cli)?;

    if !normalized.is_valid() {
        return Err(invalid_contract_error(&normalized));
    }
    if args.require_approved {
        require_approved(&normalized.contract)?;
    }

    let doc = &normalized.contract;
    match format {
        OutputFormat::Json => {
            let body = ValidReport {
                valid: true,
                status: doc.status.as_str(),
                maturity: doc.maturity.as_str(),
                targets: doc.targets.len(),
            };
            crate::output::emit_json(&body, &doc.warnings)?;
        }
        OutputFormat::Text => {
            crate::output::stdoutln!(
                "OK: {} normalizes cleanly (status={}, maturity={}, targets={})",
                contract::CONTRACT_FILENAME,
                doc.status.as_str(),
                doc.maturity.as_str(),
                doc.targets.len()
            );
            for warning in &doc.warnings {
                crate::output::stdoutln!("warning: {warning}");
            }
        }
    }
    Ok(())
}

/// The `contract validate --json` success body (no canonical document — the
/// gate reports only that the config is valid, plus a small summary).
#[derive(Serialize)]
struct ValidReport {
    valid: bool,
    status: &'static str,
    maturity: &'static str,
    targets: usize,
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

/// An invalid config is a caller-fixable (exit-1) error carrying every problem;
/// no canonical body is emitted (gate on the exit code).
fn invalid_contract_error(normalized: &Normalized) -> CliError {
    let problems = &normalized.problems.errors;
    let message = format!(
        "{} would not normalize: {} problem(s)",
        contract::CONTRACT_FILENAME,
        problems.len()
    );
    CliError::user("invalid_contract", message).with_problems(problems.clone())
}

fn require_approved(doc: &Contract) -> Result<(), CliError> {
    if doc.status == ossctl_core::contract::schema::Status::Approved {
        return Ok(());
    }
    Err(CliError::user(
        "not_approved",
        format!(
            "contract is status '{}', not 'approved' — a mutating member refuses a draft; review \
             it and set status: approved",
            doc.status.as_str()
        ),
    )
    .with_invalid_value(doc.status.as_str()))
}

fn render_show_text(doc: &Contract) -> Result<(), CliError> {
    let ecos = if doc.ecosystems.is_empty() {
        "[]".to_string()
    } else {
        doc.ecosystems
            .iter()
            .map(|e| e.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    crate::output::stdoutln!("schema_version: {}", doc.schema_version);
    crate::output::stdoutln!("status:         {}", doc.status.as_str());
    crate::output::stdoutln!("maturity:       {}", doc.maturity.as_str());
    crate::output::stdoutln!("ecosystems:     {ecos}");
    crate::output::stdoutln!("targets:        {}", doc.targets.len());
    crate::output::stdoutln!("versioning:     {}", doc.versioning.as_str());
    crate::output::stdoutln!("license:        {}", doc.license);
    for w in &doc.warnings {
        crate::output::stdoutln!("warning:        {w}");
    }
    Ok(())
}
