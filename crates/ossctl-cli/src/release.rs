//! `ossctl release …` handlers.
//!
//! Stub at founding: the release engine lives in `ossctl-core::release` and
//! lands in the `release-engine` unit. The argument shapes are real so the
//! surface and `--help` are accurate; each handler returns `not_implemented`.

use clap::Args;

use crate::cli::ReleaseAction;
use crate::error::CliError;
use crate::output::OutputFormat;

/// Arguments for `release plan`.
#[derive(Args, Debug)]
pub struct PlanArgs {
    /// Repository root to plan a release for (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<std::path::PathBuf>,
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

/// Dispatch a `release` subcommand to its (stub) handler.
pub fn dispatch(action: ReleaseAction, _format: OutputFormat) -> Result<(), CliError> {
    let verb = match action {
        ReleaseAction::Plan(_) => "release plan",
        ReleaseAction::Cut(_) => "release cut",
        ReleaseAction::Resume(_) => "release resume",
        ReleaseAction::Verify(_) => "release verify",
        ReleaseAction::Show(_) => "release show",
        ReleaseAction::List => "release list",
        ReleaseAction::Abandon(_) => "release abandon",
    };
    Err(CliError::not_implemented(verb))
}
