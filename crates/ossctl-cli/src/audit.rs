//! `ossctl audit` handler.
//!
//! Stub at founding: the scorer lives in `ossctl-core::audit` and lands in the
//! `audit-command` unit.

use clap::Args;

use crate::error::CliError;
use crate::output::OutputFormat;

/// Arguments for `ossctl audit`.
#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Repository root to score (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<std::path::PathBuf>,
}

/// `ossctl audit` — score release readiness into a gap report.
pub fn run(_args: &AuditArgs, _format: OutputFormat) -> Result<(), CliError> {
    Err(CliError::not_implemented("audit"))
}
