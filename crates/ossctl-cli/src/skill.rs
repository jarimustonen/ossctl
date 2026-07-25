//! `ossctl skill …` handlers (`AGENTS-AI-FIRST-CLI.md` §15–§17).
//!
//! Stub at founding: the 10 bundled `/oss-*` skills relocate into
//! `crates/ossctl-cli/skills/<name>/SKILL.template.md` and install via this
//! subcommand in the `skill-subcommand` / `prose-skills` units. The argument
//! shapes are real; each handler returns `not_implemented`.

use std::path::PathBuf;

use clap::Args;

use crate::cli::SkillAction;
use crate::error::CliError;
use crate::output::OutputFormat;

/// Arguments for `skill install`.
#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Skill name (see `skill list`). Omit to install every bundled skill.
    #[arg(value_name = "NAME")]
    pub name: Option<String>,
    /// Override the install destination (default: the Claude skills dir).
    #[arg(long, value_name = "PATH")]
    pub dest: Option<PathBuf>,
    /// Overwrite an existing skill at the destination.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `skill print`.
#[derive(Args, Debug)]
pub struct PrintArgs {
    /// Skill name to stream (see `skill list`).
    #[arg(value_name = "NAME")]
    pub name: String,
}

/// Dispatch a `skill` subcommand to its (stub) handler.
pub fn dispatch(action: SkillAction, _format: OutputFormat) -> Result<(), CliError> {
    let verb = match action {
        SkillAction::List => "skill list",
        SkillAction::Install(_) => "skill install",
        SkillAction::Print(_) => "skill print",
    };
    Err(CliError::not_implemented(verb))
}
