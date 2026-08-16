//! Read-only inspection of the locations `ossctl` resolves at runtime.
//!
//! `OSS-RELEASE.md` is the project's release contract, not an `ossctl` tool
//! config file (ADR-0001 / ADR-0003). `config` therefore reports that real
//! contract location and the release journal root; it does not invent a
//! home-directory config or unsupported `OSSCTL_*` settings.

use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use ossctl_core::release::journal::JournalPaths;

use crate::error::CliError;
use crate::output::OutputFormat;
use crate::sys::RealGitRepo;

/// Location selectors shared by `config path` and `config show`.
#[derive(Args, Debug)]
pub struct ConfigArgs {
    /// Repository root used to locate OSS-RELEASE.md and the git-common-dir
    /// release journal (default: current directory).
    #[arg(long, value_name = "PATH")]
    pub repo_root: Option<PathBuf>,
    /// Release-journal root used by `release` commands instead of
    /// `git-common-dir/ossctl/releases`. This is a per-invocation override for
    /// CI and debugging, not a persistent ossctl setting.
    #[arg(long, value_name = "PATH")]
    pub journal_dir: Option<PathBuf>,
}

/// `config` verbs.
#[derive(clap::Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the project-contract and release-journal locations ossctl resolves.
    Path(ConfigArgs),
    /// Show resolved locations and their provenance.
    Show(ConfigArgs),
}

/// Dispatch a read-only `config` operation.
pub fn dispatch(action: ConfigAction, format: OutputFormat) -> Result<(), CliError> {
    match action {
        ConfigAction::Path(args) => path(&args, format),
        ConfigAction::Show(args) => show(&args, format),
    }
}

/// `ossctl config path` — print every location the current invocation resolves.
fn path(args: &ConfigArgs, format: OutputFormat) -> Result<(), CliError> {
    let report = resolve(args)?;
    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &[])?,
        OutputFormat::Text => {
            println!("contract: {}", report.contract_path.value);
            match &report.journal_dir.value {
                Some(path) => println!("release_journal: {path}"),
                None => println!(
                    "release_journal: unavailable ({})",
                    report.journal_dir.detail
                ),
            }
        }
    }
    Ok(())
}

/// `ossctl config show` — expose the effective values and per-key provenance.
fn show(args: &ConfigArgs, format: OutputFormat) -> Result<(), CliError> {
    let report = resolve(args)?;
    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &[])?,
        OutputFormat::Text => {
            println!("contract_path: {}", report.contract_path.value);
            println!("  source: {}", report.contract_path.source);
            println!("  detail: {}", report.contract_path.detail);
            match &report.journal_dir.value {
                Some(path) => println!("journal_dir: {path}"),
                None => println!("journal_dir: unavailable"),
            }
            println!("  source: {}", report.journal_dir.source);
            println!("  detail: {}", report.journal_dir.detail);
            if report.git_environment.is_empty() {
                println!("git_environment: none");
            } else {
                for value in &report.git_environment {
                    println!(
                        "git_environment.{}: {} ({})",
                        value.key, value.value, value.source
                    );
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ConfigReport {
    /// `OSS-RELEASE.md` is a project contract, not an ossctl-owned settings file.
    tool_config_file: Option<String>,
    contract_path: ResolvedPath<String>,
    journal_dir: ResolvedPath<Option<String>>,
    /// Git environment inputs that can affect git-common-dir resolution. These
    /// are shown only when set; ossctl defines no `OSSCTL_*` environment config.
    git_environment: Vec<EnvironmentValue>,
}

#[derive(Debug, Serialize)]
struct ResolvedPath<T> {
    value: T,
    source: &'static str,
    detail: String,
}

#[derive(Debug, Serialize)]
struct EnvironmentValue {
    key: &'static str,
    value: String,
    source: String,
}

fn resolve(args: &ConfigArgs) -> Result<ConfigReport, CliError> {
    let (repo_root, repo_source, repo_detail) = match &args.repo_root {
        Some(path) => (path.clone(), "flag", "selected by --repo-root".to_string()),
        None => (
            std::env::current_dir()
                .map_err(|e| CliError::system("io_error", format!("cannot resolve cwd: {e}")))?,
            "default",
            "current directory (the default --repo-root)".to_string(),
        ),
    };

    let contract_path = ResolvedPath {
        value: repo_root
            .join(ossctl_core::contract::CONTRACT_FILENAME)
            .display()
            .to_string(),
        source: repo_source,
        detail: repo_detail,
    };

    let journal_dir = match &args.journal_dir {
        Some(path) => ResolvedPath {
            value: Some(path.display().to_string()),
            source: "flag",
            detail: "selected by --journal-dir".to_string(),
        },
        None => match JournalPaths::from_git(&RealGitRepo::new(&repo_root), None) {
            Ok(paths) => ResolvedPath {
                value: Some(paths.releases_dir().display().to_string()),
                source: "git",
                detail: "derived from git rev-parse --git-common-dir".to_string(),
            },
            Err(error) => ResolvedPath {
                value: None,
                source: "unresolved",
                detail: format!(
                    "requires a Git repository to derive git-common-dir/ossctl/releases: {error}"
                ),
            },
        },
    };

    let git_environment = ["GIT_DIR", "GIT_COMMON_DIR"]
        .into_iter()
        .filter_map(|key| {
            std::env::var(key).ok().map(|value| EnvironmentValue {
                key,
                source: format!("env:{key}"),
                value,
            })
        })
        .collect();

    Ok(ConfigReport {
        tool_config_file: None,
        contract_path,
        journal_dir,
        git_environment,
    })
}
