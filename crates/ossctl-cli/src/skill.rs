//! `ossctl skill …` — the companion-skill installer (`AGENTS-AI-FIRST-CLI.md`
//! §15–§17).
//!
//! The `/oss-*` skills are the agent's *operating manual* for driving `ossctl`
//! in multi-step workflows. They ship **inside this binary** (embedded with
//! `include_str!`) so they version in lockstep with the CLI surface they
//! reference: the binary is the source of truth, the skill follows it (§17).
//!
//! Each bundled skill is a `SKILL.template.md` carrying `{{CLI_VERSION}}` /
//! `{{SKILL_SCHEMA_VERSION}}` tokens that are substituted for the **running**
//! binary's versions at `print`/`install` time — so a printed or installed
//! skill can never claim a `cli_version` other than the binary that produced
//! it. This mirrors how `orchestratectl`/`issuectl` ship their skill families.

use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use serde::Serialize;

use crate::cli::SkillAction;
use crate::error::CliError;
use crate::output::{emit_json, OutputFormat};

/// The skill-payload format version (`AGENTS-AI-FIRST-CLI.md` §17). Independent
/// of the tool's data `schema_version`: it versions the *skill file shape*
/// (frontmatter keys, token vocabulary) so an agent can detect a breaking
/// change to the skill format without coupling it to the contract schema.
pub const SKILL_SCHEMA_VERSION: u32 = 1;

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// One skill bundled into the binary. `template` is the raw
/// `SKILL.template.md` (with unsubstituted `{{…}}` tokens); rendering happens
/// at `print`/`install` time via [`render`].
pub struct BundledSkill {
    /// Skill name — the `/oss-*` slug and the on-disk directory name.
    pub name: &'static str,
    /// One-line description shown by `skill list`.
    pub description: &'static str,
    /// The `SKILL.template.md` source, embedded at compile time.
    pub template: &'static str,
    /// The repo-relative path the template lives at (for `print`'s
    /// `path_in_repo`, so an agent can locate the source).
    pub path_in_repo: &'static str,
}

/// The catalog of bundled skills. This issue wires the **mechanism** plus the
/// first template(s); the remaining `/oss-*` members land via `migrate-oss-init`
/// and `prose-skills`. Adding a skill is a one-row `include_str!` here.
pub const CATALOG: &[BundledSkill] = &[
    BundledSkill {
        name: "oss-release",
        description:
            "Orchestrator/router of the /oss-* family: read the contract, score readiness, \
             sequence members, cut the release.",
        template: include_str!("../skills/oss-release/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-release/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-readiness",
        description:
            "Score OSS-release readiness and turn the gap report into a prioritized action list \
             (a thin skill over `ossctl audit`).",
        template: include_str!("../skills/oss-readiness/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-readiness/SKILL.template.md",
    },
];

/// Which agent runtime(s) `skill install` targets. Each selects a well-known
/// skills directory under `$HOME` *and* the on-disk file shape that runtime
/// expects; `--dest` overrides the directory (not the shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
#[value(rename_all = "kebab-case")]
pub enum Agent {
    /// `~/.claude/skills/<name>/SKILL.md` — Claude Code (the default).
    #[default]
    Claude,
    /// `~/.codex/prompts/<name>.md` — Codex.
    Codex,
    /// Install into every known runtime.
    All,
}

impl Agent {
    /// The concrete runtimes this selector expands to.
    fn runtimes(self) -> &'static [Runtime] {
        match self {
            Agent::Claude => &[Runtime::Claude],
            Agent::Codex => &[Runtime::Codex],
            Agent::All => &[Runtime::Claude, Runtime::Codex],
        }
    }
}

/// A single concrete runtime (never `All`) — carries the per-runtime path shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Runtime {
    Claude,
    Codex,
}

impl Runtime {
    /// A short label for the install report / warnings.
    fn label(self) -> &'static str {
        match self {
            Runtime::Claude => "claude",
            Runtime::Codex => "codex",
        }
    }

    /// The full on-disk path this runtime expects for skill `name`, rooted at
    /// `root` (the runtime's `$HOME` dir, or a `--dest` override).
    fn path_under(self, root: &Path, name: &str) -> PathBuf {
        match self {
            // Claude: a directory per skill, canonical `SKILL.md` inside.
            Runtime::Claude => root.join(name).join("SKILL.md"),
            // Codex: a flat `<name>.md` prompt file.
            Runtime::Codex => root.join(format!("{name}.md")),
        }
    }

    /// The default `$HOME`-rooted directory for this runtime.
    fn default_root(self, home: &Path) -> PathBuf {
        match self {
            Runtime::Claude => home.join(".claude/skills"),
            Runtime::Codex => home.join(".codex/prompts"),
        }
    }
}

/// Arguments for `skill install`.
#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Skill name (see `skill list`). Omit to install every bundled skill.
    #[arg(value_name = "NAME")]
    pub name: Option<String>,
    /// Agent runtime to install into (default: `claude`).
    #[arg(long, value_enum, default_value_t = Agent::Claude)]
    pub agent: Agent,
    /// Override the destination directory (the runtime's file shape still
    /// applies under it). Incompatible with `--agent all`.
    #[arg(long, value_name = "PATH")]
    pub dest: Option<PathBuf>,
    /// Overwrite an existing skill even when the on-disk copy is newer than the
    /// running binary (which §17 otherwise refuses).
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

/// Dispatch a `skill` subcommand to its handler.
pub fn dispatch(action: SkillAction, format: OutputFormat) -> Result<(), CliError> {
    match action {
        SkillAction::List => list(format),
        SkillAction::Install(args) => install(&args, format),
        SkillAction::Print(args) => print(&args, format),
    }
}

/// Look a skill up by name, or return an `unknown_skill` §10 error carrying the
/// accepted set (§4) so an agent gets a self-correcting envelope.
fn lookup(name: &str) -> Result<&'static BundledSkill, CliError> {
    CATALOG.iter().find(|s| s.name == name).ok_or_else(|| {
        let known: Vec<&str> = CATALOG.iter().map(|s| s.name).collect();
        CliError::user("unknown_skill", format!("no bundled skill named `{name}`"))
            .with_invalid_value(name)
            .with_expected(serde_json::json!(known))
    })
}

/// Substitute the `{{…}}` tokens for the **running** binary's versions. This is
/// the pinning seam (§17): the rendered text always carries *this* binary's
/// `cli_version`, never a stale one.
fn render(skill: &BundledSkill) -> String {
    skill
        .template
        .replace("{{CLI_VERSION}}", CLI_VERSION)
        .replace(
            "{{SKILL_SCHEMA_VERSION}}",
            &SKILL_SCHEMA_VERSION.to_string(),
        )
}

// ── skill list ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ListEntry<'a> {
    name: &'a str,
    description: &'a str,
    cli_version: &'a str,
    schema_version: u32,
    path_in_repo: &'a str,
}

#[derive(Serialize)]
struct ListPayload<'a> {
    skills: Vec<ListEntry<'a>>,
}

fn list(format: OutputFormat) -> Result<(), CliError> {
    let skills: Vec<ListEntry> = CATALOG
        .iter()
        .map(|s| ListEntry {
            name: s.name,
            description: s.description,
            cli_version: CLI_VERSION,
            schema_version: SKILL_SCHEMA_VERSION,
            path_in_repo: s.path_in_repo,
        })
        .collect();

    match format {
        OutputFormat::Json => emit_json(&ListPayload { skills }, &[])?,
        OutputFormat::Text => {
            if skills.is_empty() {
                println!("no skills bundled");
            } else {
                for s in &skills {
                    println!("{}  (cli {})  {}", s.name, s.cli_version, s.description);
                }
            }
        }
    }
    Ok(())
}

// ── skill print ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct PrintPayload<'a> {
    name: &'a str,
    cli_version: &'a str,
    /// The skill-format version (§17). Named `schema_version_skill` in the
    /// payload to distinguish it from the envelope's data `schema_version`.
    schema_version_skill: u32,
    content: String,
    path_in_repo: &'a str,
}

fn print(args: &PrintArgs, format: OutputFormat) -> Result<(), CliError> {
    let skill = lookup(&args.name)?;
    let content = render(skill);
    match format {
        OutputFormat::Json => emit_json(
            &PrintPayload {
                name: skill.name,
                cli_version: CLI_VERSION,
                schema_version_skill: SKILL_SCHEMA_VERSION,
                content,
                path_in_repo: skill.path_in_repo,
            },
            &[],
        )?,
        // Text: byte-identical to what `install` writes to disk — no "rendered"
        // vs "raw" distinction (§16). `print!` (not `println!`) so a file that
        // ends in exactly one newline is reproduced faithfully.
        OutputFormat::Text => print!("{content}"),
    }
    Ok(())
}

// ── skill install ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct InstalledEntry {
    name: String,
    /// The runtime this copy was written for (`claude` / `codex`).
    agent: &'static str,
    dest_path: String,
    cli_version: String,
    schema_version: u32,
}

#[derive(Serialize)]
struct InstallPayload {
    installed: Vec<InstalledEntry>,
}

/// One resolved (skill, runtime, path) install target.
struct PlanEntry<'a> {
    skill: &'a BundledSkill,
    runtime: Runtime,
    path: PathBuf,
}

fn install(args: &InstallArgs, format: OutputFormat) -> Result<(), CliError> {
    // `--dest` names a single directory, so it cannot serve two runtime shapes.
    if args.dest.is_some() && args.agent == Agent::All {
        return Err(CliError::user(
            "invalid_arguments",
            "--dest is incompatible with --agent all (one directory cannot hold both runtime shapes)",
        ));
    }

    // Which skills: the named one, or the whole catalog.
    let targets: Vec<&BundledSkill> = match &args.name {
        Some(name) => vec![lookup(name)?],
        None => CATALOG.iter().collect(),
    };

    // Resolve $HOME lazily — only needed when no `--dest` override is given.
    let home = home_dir();

    // Build the full plan before touching disk.
    let mut plan = Vec::new();
    for &runtime in args.agent.runtimes() {
        let root = match &args.dest {
            Some(dest) => dest.clone(),
            None => runtime.default_root(home.as_deref().ok_or_else(home_error)?),
        };
        for skill in &targets {
            plan.push(PlanEntry {
                skill,
                runtime,
                path: runtime.path_under(&root, skill.name),
            });
        }
    }

    // Preflight: classify every target's drift up front so a §17 refusal fails
    // the whole command *before* any partial write.
    let mut warnings = Vec::new();
    for entry in &plan {
        if let Some(w) = check_drift(&entry.path, args.force)? {
            warnings.push(w);
        }
    }

    // All clear — write every target.
    let mut installed = Vec::new();
    for entry in &plan {
        let content = render(entry.skill);
        if let Some(parent) = entry.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CliError::system(
                    "install_failed",
                    format!("could not create `{}`: {e}", parent.display()),
                )
            })?;
        }
        std::fs::write(&entry.path, &content).map_err(|e| {
            CliError::system(
                "install_failed",
                format!("could not write `{}`: {e}", entry.path.display()),
            )
        })?;
        installed.push(InstalledEntry {
            name: entry.skill.name.to_string(),
            agent: entry.runtime.label(),
            dest_path: entry.path.display().to_string(),
            cli_version: CLI_VERSION.to_string(),
            schema_version: SKILL_SCHEMA_VERSION,
        });
    }

    match format {
        OutputFormat::Json => emit_json(&InstallPayload { installed }, &warnings)?,
        OutputFormat::Text => {
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            for e in &installed {
                println!("installed {} ({}) → {}", e.name, e.agent, e.dest_path);
            }
        }
    }
    Ok(())
}

/// Resolve `$HOME` as a path, or `None` when unset/empty.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

fn home_error() -> CliError {
    CliError::system(
        "no_home_dir",
        "cannot resolve the skills directory: $HOME is unset (pass --dest <PATH>)",
    )
}

/// Inspect an existing on-disk skill and apply the §17 drift policy. Returns
/// `Ok(Some(warning))` when install may proceed but the caller should be warned,
/// `Ok(None)` when there is nothing to say, and `Err(..)` when §17 refuses the
/// overwrite (on-disk copy newer than the binary) without `--force`.
fn check_drift(path: &Path, force: bool) -> Result<Option<String>, CliError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(s) => s,
        // No existing file → nothing to compare; a clean first install.
        Err(_) => return Ok(None),
    };
    // Present but unparseable frontmatter: treat as a refusal absent --force —
    // we will not silently clobber a file we cannot reason about.
    let Some(on_disk) = frontmatter_cli_version(&existing) else {
        if force {
            return Ok(Some(format!(
                "overwriting `{}`: existing file has no readable cli_version",
                path.display()
            )));
        }
        return Err(CliError::user(
            "skill_install_conflict",
            format!(
                "`{}` exists but its cli_version is unreadable; pass --force to overwrite",
                path.display()
            ),
        ));
    };

    match compare_versions(&on_disk, CLI_VERSION) {
        // On-disk older → §17 says proceed with a warning.
        Some(std::cmp::Ordering::Less) => Ok(Some(format!(
            "upgrading `{}` from cli_version {on_disk} to {CLI_VERSION}",
            path.display()
        ))),
        // Equal → idempotent re-install, nothing to report.
        Some(std::cmp::Ordering::Equal) => Ok(None),
        // On-disk newer → refuse unless forced (agent upgraded ahead of binary).
        Some(std::cmp::Ordering::Greater) => {
            if force {
                Ok(Some(format!(
                    "downgrading `{}` from cli_version {on_disk} to {CLI_VERSION} (--force)",
                    path.display()
                )))
            } else {
                Err(CliError::user(
                    "skill_version_mismatch",
                    format!(
                        "`{}` has cli_version {on_disk}, newer than this binary ({CLI_VERSION}); \
                         pass --force to overwrite",
                        path.display()
                    ),
                ))
            }
        }
        // Unparseable version string on disk — same conservative refusal.
        None => {
            if force {
                Ok(Some(format!(
                    "overwriting `{}`: existing cli_version `{on_disk}` is unparseable",
                    path.display()
                )))
            } else {
                Err(CliError::user(
                    "skill_install_conflict",
                    format!(
                        "`{}` has an unparseable cli_version `{on_disk}`; pass --force to overwrite",
                        path.display()
                    ),
                ))
            }
        }
    }
}

/// Extract the `cli_version:` value from a SKILL.md YAML frontmatter block
/// without a full YAML parser (the frontmatter shape is fixed and simple). Only
/// scans the leading `---`-delimited block. Returns the unquoted value.
fn frontmatter_cli_version(text: &str) -> Option<String> {
    let body = text.strip_prefix("---")?;
    // Frontmatter ends at the next line that is exactly `---`.
    let end = body.find("\n---")?;
    let block = &body[..end];
    for line in block.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("cli_version:") {
            let v = rest.trim().trim_matches(|c| c == '"' || c == '\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Compare two dotted numeric version strings (e.g. `0.1.0`) component-wise.
/// `None` if either side has a non-numeric component — the caller then treats
/// the comparison as a refusable conflict rather than guessing. Kept dependency
/// free (no `semver` crate) since bundled `cli_version`s are plain `x.y.z`.
fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let parse =
        |s: &str| -> Option<Vec<u64>> { s.split('.').map(|p| p.parse::<u64>().ok()).collect() };
    let (a, b) = (parse(a)?, parse(b)?);
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        let ord = x.cmp(&y);
        if ord != std::cmp::Ordering::Equal {
            return Some(ord);
        }
    }
    Some(std::cmp::Ordering::Equal)
}
