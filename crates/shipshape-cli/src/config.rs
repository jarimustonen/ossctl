//! Read-only inspection of the locations `shipshape` resolves at runtime.
//!
//! `OSS-RELEASE.md` is the project's release contract, not an `shipshape` tool
//! config file (ADR-0001 / ADR-0003). `config` therefore reports that real
//! contract location and the release journal root; it does not invent a
//! home-directory config or unsupported `SHIPSHAPE_*` settings.

use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use shipshape_core::release::journal::JournalPaths;

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
    /// CI and debugging, not a persistent shipshape setting.
    #[arg(long, value_name = "PATH")]
    pub journal_dir: Option<PathBuf>,
}

/// `config` verbs.
#[derive(clap::Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the project-contract and release-journal locations shipshape resolves.
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

/// `shipshape config path` — print every location the current invocation resolves.
fn path(args: &ConfigArgs, format: OutputFormat) -> Result<(), CliError> {
    let report = resolve(args)?;
    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &[])?,
        OutputFormat::Text => {
            crate::output::stdoutln!("contract_path: {}", report.contract_path.value)?;
            match &report.journal_dir.value {
                Some(path) => crate::output::stdoutln!("journal_dir: {path}")?,
                None => crate::output::stdoutln!(
                    "journal_dir: unavailable ({})",
                    report.journal_dir.detail
                )?,
            }
        }
    }
    Ok(())
}

/// `shipshape config show` — expose the effective values and per-key provenance.
fn show(args: &ConfigArgs, format: OutputFormat) -> Result<(), CliError> {
    let report = resolve(args)?;
    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &[])?,
        OutputFormat::Text => {
            crate::output::stdoutln!("contract_path: {}", report.contract_path.value)?;
            crate::output::stdoutln!("  source: {}", report.contract_path.source)?;
            crate::output::stdoutln!("  detail: {}", report.contract_path.detail)?;
            crate::output::stdoutln!("  lossy: {}", report.contract_path.lossy)?;
            match &report.journal_dir.value {
                Some(path) => crate::output::stdoutln!("journal_dir: {path}")?,
                None => crate::output::stdoutln!("journal_dir: unavailable")?,
            }
            crate::output::stdoutln!("  source: {}", report.journal_dir.source)?;
            crate::output::stdoutln!("  detail: {}", report.journal_dir.detail)?;
            crate::output::stdoutln!("  lossy: {}", report.journal_dir.lossy)?;
            if report.git_environment.is_empty() {
                crate::output::stdoutln!("git_environment: none")?;
            } else {
                for value in &report.git_environment {
                    crate::output::stdoutln!(
                        "git_environment.{}: {} ({}, lossy={})",
                        value.key,
                        value.value,
                        value.source,
                        value.lossy
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ConfigReport {
    contract_path: ResolvedPath<String>,
    journal_dir: ResolvedPath<Option<String>>,
    /// Git environment inputs that affected this invocation's git-common-dir
    /// resolution. shipshape defines no `SHIPSHAPE_*` environment configuration.
    git_environment: Vec<EnvironmentValue>,
}

#[derive(Debug, Serialize)]
struct ResolvedPath<T> {
    value: T,
    source: Source,
    detail: String,
    /// JSON cannot represent arbitrary Unix path bytes. A true value means
    /// `value` is a lossy display string and must not be copied into an argv.
    lossy: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Source {
    Default,
    Flag,
    Git,
    Unresolved,
}

impl std::fmt::Display for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Default => "default",
            Self::Flag => "flag",
            Self::Git => "git",
            Self::Unresolved => "unresolved",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Serialize)]
struct EnvironmentValue {
    key: &'static str,
    value: String,
    source: String,
    lossy: bool,
}

fn resolve(args: &ConfigArgs) -> Result<ConfigReport, CliError> {
    let (repo_root, repo_source, repo_detail) = match &args.repo_root {
        Some(path) => (
            path.clone(),
            Source::Flag,
            "selected by --repo-root".to_string(),
        ),
        None => (
            std::env::current_dir()
                .map_err(|e| CliError::system("io_error", format!("cannot resolve cwd: {e}")))?,
            Source::Default,
            "current directory (the default --repo-root)".to_string(),
        ),
    };

    let contract_display =
        display_path(&repo_root.join(shipshape_core::contract::CONTRACT_FILENAME));
    let contract_path = ResolvedPath {
        value: contract_display.value,
        source: repo_source,
        detail: repo_detail,
        lossy: contract_display.lossy,
    };

    let journal_dir = match &args.journal_dir {
        Some(path) => {
            let display = display_path(path);
            ResolvedPath {
                value: Some(display.value),
                source: Source::Flag,
                detail: "selected by --journal-dir".to_string(),
                lossy: display.lossy,
            }
        }
        None => match JournalPaths::from_git(&RealGitRepo::new(&repo_root), None) {
            Ok(paths) => {
                let display = display_path(paths.releases_dir());
                ResolvedPath {
                    value: Some(display.value),
                    source: Source::Git,
                    detail: "derived from git rev-parse --git-common-dir".to_string(),
                    lossy: display.lossy,
                }
            }
            Err(error) => ResolvedPath {
                value: None,
                source: Source::Unresolved,
                detail: format!("could not derive git-common-dir/ossctl/releases: {error}"),
                lossy: false,
            },
        },
    };

    // Only the Git-derived journal branch consulted these process inputs. Git
    // receives them unchanged because `RealGitRepo` inherits its environment.
    let git_environment = if args.journal_dir.is_none() {
        ["GIT_DIR", "GIT_COMMON_DIR"]
            .into_iter()
            .filter_map(|key| {
                std::env::var_os(key).map(|value| {
                    let rendered = value.to_string_lossy();
                    EnvironmentValue {
                        key,
                        source: format!("env:{key}"),
                        lossy: matches!(rendered, std::borrow::Cow::Owned(_)),
                        value: rendered.into_owned(),
                    }
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(ConfigReport {
        contract_path,
        journal_dir,
        git_environment,
    })
}

struct DisplayPath {
    value: String,
    lossy: bool,
}

fn display_path(path: &std::path::Path) -> DisplayPath {
    let rendered = path.as_os_str().to_string_lossy();
    DisplayPath {
        lossy: matches!(rendered, std::borrow::Cow::Owned(_)),
        value: rendered.into_owned(),
    }
}
