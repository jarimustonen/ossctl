//! `contract show` / `contract validate` handlers.
//!
//! Stub at founding: the normalizer lives in `ossctl-core::contract` and lands
//! in the `contract-command` unit. Both handlers parse their arguments (so the
//! surface and `--help` are real) and then return a clean `not_implemented`
//! envelope.

use clap::Args;

use crate::error::CliError;
use crate::output::OutputFormat;

/// Shared repo-location flag for the contract handlers.
#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Repository root containing OSS-RELEASE.md (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<std::path::PathBuf>,
    /// Require the contract to be in the approved state before emitting it.
    #[arg(long)]
    pub require_approved: bool,
}

/// Arguments for the check-only `contract validate` gate.
#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Repository root containing OSS-RELEASE.md (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<std::path::PathBuf>,
}

/// `contract show` — normalize and emit the canonical contract.
pub fn show(_args: &ShowArgs, _format: OutputFormat) -> Result<(), CliError> {
    Err(CliError::not_implemented("contract show"))
}

/// `contract validate` — run the normalization pipeline and emit pass/fail.
pub fn validate(_args: &ValidateArgs, _format: OutputFormat) -> Result<(), CliError> {
    Err(CliError::not_implemented("contract validate"))
}
