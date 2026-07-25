//! `ossctl facts` handler.
//!
//! Stub at founding: the detector lives in `ossctl-core::facts` and lands in
//! the `facts-command` unit.

use clap::Args;

use crate::error::CliError;
use crate::output::OutputFormat;

/// Arguments for `ossctl facts`.
#[derive(Args, Debug)]
pub struct FactsArgs {
    /// Repository root to inspect (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<std::path::PathBuf>,
}

/// `ossctl facts` — detect deterministic repo facts.
pub fn run(_args: &FactsArgs, _format: OutputFormat) -> Result<(), CliError> {
    Err(CliError::not_implemented("facts"))
}
