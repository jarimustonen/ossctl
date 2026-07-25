//! Clap-based CLI dispatch and the `version` handler.
//!
//! The full noun-verb taxonomy (ADR-0001 §1) is wired here. At founding only
//! `version` and `doctor` do real work; every other subcommand routes to a stub
//! that returns a clean `not_implemented` error envelope (never a panic).

use std::process::ExitCode;

use clap::{ColorChoice, Parser, Subcommand};
use serde::Serialize;

use crate::error::{CliError, ExitKind};
use crate::output::OutputFormat;

const GIT_COMMIT: &str = env!("OSSCTL_GIT_COMMIT");
const CARGO_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(
    name = "ossctl",
    version = CARGO_VERSION,
    about = "Release & readiness coordinator for the /oss-* skill family.",
    long_about = "ossctl takes any repo to OSS release quality and cuts releases from it. \
It owns the OSS-RELEASE.md contract normalizer, repo-fact detection, the readiness \
audit, and the resumable per-ecosystem release-cut engine. The prose /oss-* skills \
are thin callers of this binary.",
    disable_help_subcommand = true,
    disable_version_flag = true,
    color = ColorChoice::Never,
)]
struct Cli {
    /// Emit a structured JSON envelope on stdout instead of human-readable
    /// text. Format is chosen only by this flag, never by TTY detection
    /// (AGENTS-AI-FIRST-CLI §9).
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Read/validate the OSS-RELEASE.md release contract.
    Contract {
        #[command(subcommand)]
        action: ContractAction,
    },
    /// Detect deterministic repo facts (ecosystems, packages, CI, tags).
    Facts(crate::facts::FactsArgs),
    /// Score release readiness → gap report (read-only).
    Audit(crate::audit::AuditArgs),
    /// Plan, cut, resume, verify, show, list, or abandon a release run.
    Release {
        #[command(subcommand)]
        action: ReleaseAction,
    },
    /// List, install, or print the companion /oss-* AI-skills.
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Read-only self-diagnostic (`--fix` applies the safe subset).
    Doctor(crate::doctor::DoctorArgs),
    /// Show version, commit, schema versions, and bundled skills.
    Version,
}

/// `contract` verbs (ADR-0001 §1): `show` is THE normalizer (canonical,
/// defaulted, `targets`-expanded); `validate` runs the identical pipeline and
/// emits only pass/fail.
#[derive(Subcommand, Debug)]
pub enum ContractAction {
    /// Normalize and emit the canonical OSS-RELEASE.md as JSON.
    Show(crate::contract::ShowArgs),
    /// Check-only gate: validate and emit pass/fail (no document body).
    Validate(crate::contract::ValidateArgs),
}

/// `release` verbs (ADR-0001 §1) — each a distinct, guessable safety state of a
/// partially-irreversible run.
#[derive(Subcommand, Debug)]
pub enum ReleaseAction {
    /// Compute and seal a content-addressed release plan.
    Plan(crate::release::PlanArgs),
    /// Execute a sealed plan; refuses on repo drift.
    Cut(crate::release::CutArgs),
    /// Reconcile and continue an interrupted run.
    Resume(crate::release::RunIdArgs),
    /// Read-only reconcile of a run against remote registry state.
    Verify(crate::release::RunIdArgs),
    /// Query a run's progress (live) or post-mortem.
    Show(crate::release::RunIdArgs),
    /// List runs (active and past).
    List,
    /// Terminally mark a run un-resumable (journaled).
    Abandon(crate::release::AbandonArgs),
}

/// `skill` verbs (`AGENTS-AI-FIRST-CLI.md` §15–§16).
#[derive(Subcommand, Debug)]
pub enum SkillAction {
    /// List the skills bundled with this binary.
    List,
    /// Copy a skill into the agent's skill directory (all when no name).
    Install(crate::skill::InstallArgs),
    /// Stream a skill's SKILL.md to stdout without installing.
    Print(crate::skill::PrintArgs),
}

/// Parse argv, dispatch, and map the result to a process exit code.
pub fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return handle_clap_error(&e),
    };

    let format = OutputFormat::from_json_flag(cli.json);

    let result = match cli.command {
        Command::Version => cmd_version(format),
        Command::Contract { action } => match action {
            ContractAction::Show(args) => crate::contract::show(&args, format),
            ContractAction::Validate(args) => crate::contract::validate(&args, format),
        },
        Command::Facts(args) => crate::facts::run(&args, format),
        Command::Audit(args) => crate::audit::run(&args, format),
        Command::Release { action } => crate::release::dispatch(action, format),
        Command::Skill { action } => crate::skill::dispatch(action, format),
        // `doctor` owns its exit code directly (§18: exit 1 on any `fail`
        // *without* an error envelope), which does not map onto the shared
        // Result path — the `return` diverges so this arm's type is compatible.
        Command::Doctor(args) => return crate::doctor::run(&args, format),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            e.emit();
            ExitCode::from(e.kind as u8)
        }
    }
}

/// Translate a clap parse failure into either its native help/usage output
/// (exit 0 for `--help`) or the §10 error envelope on stderr.
fn handle_clap_error(e: &clap::Error) -> ExitCode {
    use clap::error::{ContextKind, ErrorKind};
    // Help is not a failure; let clap print and exit 0. `--version` is disabled
    // at the clap level, so it never surfaces here — agents use `version`.
    if matches!(
        e.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
        let _ = e.print();
        return ExitCode::SUCCESS;
    }

    // Preserve clap's full context (allowed values, usage) — §4 requires the
    // expected format to reach the caller. Drop only the TTY-dependent
    // "For more information…" trailer, which is noise in the JSON envelope.
    let message = e
        .to_string()
        .lines()
        .filter(|l| !l.trim_start().starts_with("For more information"))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let message = if message.is_empty() {
        "invalid arguments".to_string()
    } else {
        message
    };

    // Distinct codes so an agent branching on `code` gets a distinct fix — a
    // bad subcommand and a bad flag are not the same problem.
    let code = match e.kind() {
        ErrorKind::InvalidSubcommand => "unknown_subcommand",
        ErrorKind::UnknownArgument => "unknown_flag",
        ErrorKind::MissingRequiredArgument | ErrorKind::MissingSubcommand => "missing_argument",
        ErrorKind::InvalidValue => "invalid_value",
        _ => "invalid_arguments",
    };

    // §4: hand the caller the offending value and the accepted set as
    // structured fields when clap knows them, instead of forcing it to scrape
    // prose out of `message`.
    let mut err = CliError::user(code, message);
    if let Some(invalid) = clap_context(e, ContextKind::InvalidValue) {
        err = err.with_invalid_value(invalid);
    }
    if let Some(valid) = clap_context(e, ContextKind::ValidValue) {
        let expected: Vec<&str> = valid.split(", ").collect();
        err = err.with_expected(serde_json::json!(expected));
    }
    err.emit();
    ExitCode::from(ExitKind::User as u8)
}

/// Pull one piece of clap's structured error context (the offending value, the
/// allowed-value set, …) out of a parse failure, stringified. `None` when clap
/// did not attach that context to this error.
fn clap_context(e: &clap::Error, want: clap::error::ContextKind) -> Option<String> {
    e.context().find_map(|(kind, value)| {
        if kind == want {
            Some(value.to_string())
        } else {
            None
        }
    })
}

/// Metadata about one bundled companion skill (`AGENTS-AI-FIRST-CLI.md` §17).
/// The scaffold bundles no skills yet, so the catalog is empty; the field is
/// present and typed so the wire shape is stable when the skills land.
#[derive(Debug, Serialize)]
struct SkillCatalogEntry {
    name: &'static str,
    cli_version: &'static str,
    schema_version: u32,
}

#[derive(Debug, Serialize)]
struct VersionPayload {
    version: &'static str,
    commit: &'static str,
    schema_version: u32,
    supported_schemas: &'static [u32],
    skills: Vec<SkillCatalogEntry>,
}

/// `ossctl version` — the §10/§17 version surface. Emits `version`, `commit`,
/// `schema_version`, `supported_schemas`, and the bundled-skill catalog.
fn cmd_version(format: OutputFormat) -> Result<(), CliError> {
    let payload = VersionPayload {
        version: CARGO_VERSION,
        commit: GIT_COMMIT,
        schema_version: ossctl_core::SCHEMA_VERSION,
        supported_schemas: ossctl_core::SUPPORTED_SCHEMAS,
        // No skills bundled at founding (they land with the skill-subcommand /
        // prose-skills units). Empty, not absent — the field is the contract.
        skills: Vec::new(),
    };
    match format {
        OutputFormat::Json => crate::output::emit_json(&payload, &[])?,
        OutputFormat::Text => {
            println!("ossctl {}", payload.version);
            println!("commit:            {}", payload.commit);
            println!("schema version:    {}", payload.schema_version);
            println!(
                "supported schemas: {}",
                format_u32_list(payload.supported_schemas)
            );
            println!("bundled skills:    {}", payload.skills.len());
        }
    }
    Ok(())
}

fn format_u32_list(values: &[u32]) -> String {
    values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
