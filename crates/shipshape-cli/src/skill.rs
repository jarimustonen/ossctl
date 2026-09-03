//! `shipshape skill …` — the companion-skill installer (`AGENTS-AI-FIRST-CLI.md`
//! §15–§17).
//!
//! The `/shipshape-*` skills are the agent's *operating manual* for driving `shipshape`
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
#[derive(Debug)]
pub struct BundledSkill {
    /// Skill name — the `/shipshape-*` slug and the on-disk directory name.
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
/// first template(s); the remaining `/shipshape-*` members land via `migrate-shipshape-init`
/// and `prose-skills`. Adding a skill is a one-row `include_str!` here.
pub const CATALOG: &[BundledSkill] = &[
    BundledSkill {
        name: "shipshape-architecture",
        description:
            "Opt-in architecture docs of the /shipshape-* family: emit a matklad-style \
             ARCHITECTURE.md code map, scaffold an ADR log + docs site, act on the `docs_site` \
             config field. Never a readiness gate; ADRs come from /worktree-technical-decision.",
        template: include_str!("../skills/shipshape-architecture/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-architecture/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-init",
        description:
            "Generator of a project's OSS-RELEASE.md config: read repo facts, infer the dials, \
             write a human-reviewable draft (a thin skill over `shipshape facts` + `contract validate`).",
        template: include_str!("../skills/shipshape-init/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-init/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-release",
        description:
            "Orchestrator/router of the /shipshape-* family: read the contract, score readiness, \
             sequence members, cut the release.",
        template: include_str!("../skills/shipshape-release/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-release/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-ci",
        description:
            "Generator of a repo's contribution-quality CI: read the contract, emit the \
             tier/ecosystem-tuned GitHub Actions workflow + dep-bot/pre-commit/security-lint \
             gates (a thin skill over `shipshape contract show`).",
        template: include_str!("../skills/shipshape-ci/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-ci/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-security-policy",
        description:
            "Threat-gated generator of SECURITY.md: detect an enumerated set of threat signals \
             from repo inspection + `shipshape facts`/`contract show`, and emit a full \
             coordinated-disclosure policy when the surface warrants, else a minimal pointer.",
        template: include_str!("../skills/shipshape-security-policy/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-security-policy/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-changelog",
        description:
            "Establish + maintain CHANGELOG.md (sole writer): Keep-a-Changelog skeleton, \
             marker-anchored [Unreleased] ops, and release finalize — reads changelog.mode from \
             `shipshape contract show`, compiles fragments via `issuectl changelog`.",
        template: include_str!("../skills/shipshape-changelog/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-changelog/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-readiness",
        description:
            "Score OSS-release readiness and turn the gap report into a prioritized action list \
             (a thin skill over `shipshape audit`).",
        template: include_str!("../skills/shipshape-readiness/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-readiness/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-readme",
        description:
            "Generator of a project's README.md front door + LICENSE: read the contract \
             (license/ecosystems/targets) and facts, emit a maturity-tiered slotted README and \
             an SPDX-correct LICENSE (a thin skill over `shipshape contract show` + `facts`).",
        template: include_str!("../skills/shipshape-readme/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-readme/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-contributing",
        description:
            "Generator of a project's contributor-onboarding docs: CONTRIBUTING.md plus \
             tier-gated code of conduct, issue forms, PR template, and governance — templated \
             emission tuned to the contract (a thin skill over `shipshape contract show`).",
        template: include_str!("../skills/shipshape-contributing/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-contributing/SKILL.template.md",
    },
    BundledSkill {
        name: "shipshape-dist",
        description:
            "Generator of Rust binary-distribution infrastructure: contract-driven cargo-dist \
             config + generated release workflow, Cargo dist profile, and Homebrew tap/secret \
             operations guidance (a thin skill over `shipshape contract show` + `dist generate`).",
        template: include_str!("../skills/shipshape-dist/SKILL.template.md"),
        path_in_repo: "crates/shipshape-cli/skills/shipshape-dist/SKILL.template.md",
    },
];

/// Which agent runtime(s) `skill install` targets. Each selects a well-known
/// skills directory under the install base and the on-disk artifact form that
/// runtime expects. `--target` overrides the install base; legacy `--dest`
/// overrides the already-resolved skills directory.
///
/// When `--agent` is omitted, all maintained runtimes are selected. An explicit
/// value narrows to one runtime, or `all` selects all three explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum Agent {
    /// `~/.claude/skills/<name>/SKILL.md` — Claude Code.
    Claude,
    /// `~/.pi/agent/skills/<name>/SKILL.md` — pi.dev.
    Pi,
    /// `~/.codex/prompts/<name>.md` — Codex.
    Codex,
    /// Install into every known runtime (Claude + pi.dev + Codex).
    All,
}

impl Agent {
    /// The concrete runtimes this selector expands to.
    fn runtimes(self) -> &'static [Runtime] {
        match self {
            Agent::Claude => &[Runtime::Claude],
            Agent::Pi => &[Runtime::Pi],
            Agent::Codex => &[Runtime::Codex],
            Agent::All => &[Runtime::Claude, Runtime::Pi, Runtime::Codex],
        }
    }
}

/// The runtimes an `--agent` selection expands to. Both an omitted selector and
/// explicit `all` cover every maintained runtime, as required by canon §15.
fn selected_runtimes(agent: Option<Agent>) -> &'static [Runtime] {
    agent.unwrap_or(Agent::All).runtimes()
}

/// A single concrete runtime (never `All`) — carries the per-runtime path shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Runtime {
    /// Claude Code — `~/.claude/skills/<name>/SKILL.md`.
    Claude,
    /// pi.dev — `~/.pi/agent/skills/<name>/SKILL.md` (same shape as Claude).
    Pi,
    /// Codex — `~/.codex/prompts/<name>.md` (flat prompt file).
    Codex,
}

impl Runtime {
    /// A short label for the install report / warnings.
    fn label(self) -> &'static str {
        match self {
            Runtime::Claude => "claude",
            Runtime::Pi => "pi",
            Runtime::Codex => "codex",
        }
    }

    /// The full on-disk path this runtime expects for skill `name`, rooted at
    /// `root` (the runtime's `$HOME` dir, or a `--dest` override).
    fn path_under(self, root: &Path, name: &str) -> PathBuf {
        match self {
            // Claude and pi.dev share the shape: a directory per skill with the
            // canonical `SKILL.md` inside. pi.dev discovers `SKILL.md` from
            // `~/.pi/agent/skills/<name>/` and invokes it as `/skill:name`.
            Runtime::Claude | Runtime::Pi => root.join(name).join("SKILL.md"),
            // Codex: a flat `<name>.md` prompt file.
            Runtime::Codex => root.join(format!("{name}.md")),
        }
    }

    /// The default `$HOME`-rooted directory for this runtime.
    fn default_root(self, home: &Path) -> PathBuf {
        match self {
            Runtime::Claude => home.join(".claude/skills"),
            Runtime::Pi => home.join(".pi/agent/skills"),
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
    /// Agent runtime to install into. Omit for `all` (Claude Code, pi.dev, and
    /// Codex); pass `claude`, `pi`, or `codex` to narrow to one runtime.
    #[arg(long, value_enum)]
    pub agent: Option<Agent>,
    /// Override the install base. Runtime-native layouts are created below it:
    /// `.claude/skills`, `.pi/agent/skills`, and `.codex/prompts`.
    #[arg(long, value_name = "PATH", conflicts_with = "dest")]
    pub target: Option<PathBuf>,
    /// Compatibility override for the resolved skills directory. Unlike canonical
    /// `--target`, runtime layout prefixes are not added below this path.
    #[arg(long, value_name = "PATH", conflicts_with = "target")]
    pub dest: Option<PathBuf>,
    /// Validate and print the complete install plan without writing anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Overwrite an unmanaged, non-regular, malformed, or newer destination.
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
    if let Some(suffix) = name.strip_prefix("oss-") {
        let replacement = format!("shipshape-{suffix}");
        if CATALOG.iter().any(|skill| skill.name == replacement) {
            return Err(CliError::user(
                "skill_renamed",
                format!(
                    "bundled skill `{name}` was renamed to `{replacement}`; run `shipshape skill install {replacement}`"
                ),
            )
            .with_invalid_value(name)
            .with_expected(serde_json::json!([replacement])));
        }
    }

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

const SUPPORTED_AGENTS: &[&str] = &["claude", "pi", "codex"];
const ACCEPTED_AGENT_VALUES: &[&str] = &["claude", "pi", "codex", "all"];

#[derive(Serialize)]
struct ListEntry<'a> {
    name: &'a str,
    description: &'a str,
    cli_version: &'a str,
    schema_version: u32,
    skill_schema_version: u32,
    path_in_repo: &'a str,
    resources: &'static [&'static str],
}

#[derive(Serialize)]
struct InstallLayout {
    agent: &'static str,
    path: &'static str,
    form: &'static str,
}

#[derive(Serialize)]
struct InstallCapability {
    selection_flag: &'static str,
    default: &'static str,
    accepted_values: &'static [&'static str],
    target_flag: &'static str,
    dry_run_flag: &'static str,
    force_flag: &'static str,
    interactive: bool,
    no_clobber_default: bool,
    overwrite_requires_force: bool,
    layouts: [InstallLayout; 3],
}

#[derive(Serialize)]
struct ListPayload<'a> {
    supported_agents: &'static [&'static str],
    install: InstallCapability,
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
            skill_schema_version: SKILL_SCHEMA_VERSION,
            path_in_repo: s.path_in_repo,
            resources: &["SKILL.md"],
        })
        .collect();

    let payload = ListPayload {
        supported_agents: SUPPORTED_AGENTS,
        install: InstallCapability {
            selection_flag: "--agent",
            default: "all",
            accepted_values: ACCEPTED_AGENT_VALUES,
            target_flag: "--target",
            dry_run_flag: "--dry-run",
            force_flag: "--force",
            interactive: false,
            no_clobber_default: true,
            overwrite_requires_force: true,
            layouts: [
                InstallLayout {
                    agent: "claude",
                    path: ".claude/skills/<name>/...",
                    form: "agent-skill-tree",
                },
                InstallLayout {
                    agent: "pi",
                    path: ".pi/agent/skills/<name>/...",
                    form: "agent-skill-tree",
                },
                InstallLayout {
                    agent: "codex",
                    path: ".codex/prompts/<name>.md",
                    form: "self-contained-prompt",
                },
            ],
        },
        skills,
    };

    match format {
        OutputFormat::Json => emit_json(&payload, &[])?,
        OutputFormat::Text => {
            if payload.skills.is_empty() {
                crate::output::stdoutln!("no skills bundled")?;
            } else {
                for s in &payload.skills {
                    crate::output::stdoutln!(
                        "{}  (cli {})  {}",
                        s.name,
                        s.cli_version,
                        s.description
                    )?;
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
        // vs "raw" distinction (§16). The no-newline helper reproduces a file
        // ending in exactly one newline faithfully.
        OutputFormat::Text => crate::output::stdout!("{content}")?,
    }
    Ok(())
}

// ── skill install ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct InstalledEntry {
    name: String,
    /// The runtime this copy was written for (`claude` / `pi` / `codex`).
    agent: &'static str,
    dest_path: String,
    cli_version: String,
    schema_version: u32,
    action: &'static str,
}

#[derive(Serialize)]
struct WouldEntry {
    action: &'static str,
    resource: &'static str,
    input: WouldInput,
    known_effects: WouldEffects,
    unknown_until_apply: &'static [&'static str],
}

#[derive(Serialize)]
struct WouldInput {
    name: String,
    agent: &'static str,
    path: String,
}

#[derive(Serialize)]
struct WouldEffects {
    status: &'static str,
}

#[derive(Serialize)]
struct InstallPayload {
    dry_run: bool,
    force: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    installed: Vec<InstalledEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    would: Vec<WouldEntry>,
}

/// One resolved (skill, runtime, path) install target.
struct PlanEntry<'a> {
    skill: &'a BundledSkill,
    runtime: Runtime,
    path: PathBuf,
}

#[derive(Clone, Copy)]
enum PlannedAction {
    Install,
    Upgrade,
    Unchanged,
    Overwrite,
}

impl PlannedAction {
    fn label(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
            Self::Unchanged => "unchanged",
            Self::Overwrite => "overwrite",
        }
    }

    fn mutation(self) -> &'static str {
        match self {
            Self::Install => "create",
            Self::Upgrade | Self::Unchanged | Self::Overwrite => "update",
        }
    }
}

#[derive(Clone)]
struct Preflight {
    action: PlannedAction,
    warning: Option<String>,
}

fn install(args: &InstallArgs, format: OutputFormat) -> Result<(), CliError> {
    let targets: Vec<&BundledSkill> = match &args.name {
        Some(name) => vec![lookup(name)?],
        None => CATALOG.iter().collect(),
    };
    let plan = build_plan(args, &targets)?;
    let (preflights, warnings) = preflight_plan(&plan, args.force)?;
    let mut installed = Vec::new();
    let mut would = Vec::new();
    if args.dry_run {
        for entry in &plan {
            let action = preflights[&entry.path].action;
            would.push(WouldEntry {
                action: action.mutation(),
                resource: "skill",
                input: WouldInput {
                    name: entry.skill.name.to_string(),
                    agent: entry.runtime.label(),
                    path: entry.path.display().to_string(),
                },
                known_effects: WouldEffects {
                    status: action.label(),
                },
                unknown_until_apply: &[],
            });
        }
    } else {
        let mut written = std::collections::HashSet::new();
        for (idx, entry) in plan.iter().enumerate() {
            let action = preflights[&entry.path].action;
            if !matches!(action, PlannedAction::Unchanged) && written.insert(entry.path.clone()) {
                write_atomic(&entry.path, &render(entry.skill), idx)?;
            }
            installed.push(InstalledEntry {
                name: entry.skill.name.to_string(),
                agent: entry.runtime.label(),
                dest_path: entry.path.display().to_string(),
                cli_version: CLI_VERSION.to_string(),
                schema_version: SKILL_SCHEMA_VERSION,
                action: action.label(),
            });
        }
    }

    let payload = InstallPayload {
        dry_run: args.dry_run,
        force: args.force,
        installed,
        would,
    };
    match format {
        OutputFormat::Json => emit_json(&payload, &warnings)?,
        OutputFormat::Text => {
            for warning in &warnings {
                eprintln!("warning: {warning}");
            }
            if args.dry_run {
                for entry in &payload.would {
                    crate::output::stdoutln!(
                        "would {} {} ({}) → {}",
                        entry.known_effects.status,
                        entry.input.name,
                        entry.input.agent,
                        entry.input.path
                    )?;
                }
            } else {
                for entry in &payload.installed {
                    crate::output::stdoutln!(
                        "{} {} ({}) → {}",
                        entry.action,
                        entry.name,
                        entry.agent,
                        entry.dest_path
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn build_plan<'a>(
    args: &InstallArgs,
    targets: &[&'a BundledSkill],
) -> Result<Vec<PlanEntry<'a>>, CliError> {
    // `--target` is a HOME-like base; compatibility `--dest` names the resolved
    // skills directory itself and therefore skips runtime prefixes.
    let base = match (&args.target, &args.dest) {
        (Some(target), None) => Some(target.clone()),
        (None, Some(_)) => None,
        (None, None) => Some(home_dir().ok_or_else(home_error)?),
        (Some(_), Some(_)) => unreachable!("clap rejects --target with --dest"),
    };
    let mut plan = Vec::new();
    for &runtime in selected_runtimes(args.agent) {
        let root = args.dest.clone().unwrap_or_else(|| {
            runtime.default_root(base.as_deref().expect("install base resolved"))
        });
        for &skill in targets {
            plan.push(PlanEntry {
                skill,
                runtime,
                path: runtime.path_under(&root, skill.name),
            });
        }
    }
    Ok(plan)
}

fn preflight_plan(
    plan: &[PlanEntry<'_>],
    force: bool,
) -> Result<(std::collections::HashMap<PathBuf, Preflight>, Vec<String>), CliError> {
    let mut results = std::collections::HashMap::new();
    let mut warnings = Vec::new();
    for entry in plan {
        if !results.contains_key(&entry.path) {
            let result = check_drift(&entry.path, entry.skill, force)?;
            if let Some(warning) = &result.warning {
                warnings.push(warning.clone());
            }
            results.insert(entry.path.clone(), result);
        }
    }
    Ok((results, warnings))
}

/// Write `content` to `path` atomically: create the parent, write a sibling
/// temp file, then `rename` it into place. The rename is atomic on POSIX and
/// replaces the final component *as a whole* — so a crash mid-write never leaves
/// a truncated `SKILL.md`, and an existing final-component symlink is replaced
/// rather than followed. `idx` disambiguates concurrent temp files within one
/// invocation. (This is per-file atomic; a multi-file batch is still not one
/// transaction — see the preflight note in `install`.)
fn write_atomic(path: &Path, content: &str, idx: usize) -> Result<(), CliError> {
    let fail = |e: std::io::Error, what: &str| {
        CliError::system(
            "install_failed",
            format!("could not {what} `{}`: {e}", path.display()),
        )
    };
    let parent = path.parent().ok_or_else(|| {
        CliError::system(
            "install_failed",
            format!("destination `{}` has no parent directory", path.display()),
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|e| fail(e, "create"))?;

    // Sibling temp file (same directory → rename stays on one filesystem). The
    // pid + idx keep it unique against a concurrent installer.
    let tmp = parent.join(format!(".shipshape-skill.{}.{idx}.tmp", std::process::id()));
    std::fs::write(&tmp, content).map_err(|e| fail(e, "write"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        // Best-effort cleanup so a failed rename doesn't litter temp files.
        let _ = std::fs::remove_file(&tmp);
        fail(e, "install")
    })
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
        "cannot resolve the skill install base: $HOME is unset (pass --target <PATH> or --dest <PATH>)",
    )
}

/// Inspect an existing destination and apply §15's no-clobber rule plus §17's
/// managed-skill drift policy. A regular file is managed only when its top-level
/// `name` matches the bundled skill expected at that exact path.
fn check_drift(path: &Path, skill: &BundledSkill, force: bool) -> Result<Preflight, CliError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Preflight {
                action: PlannedAction::Install,
                warning: None,
            });
        }
        Err(e) => {
            return Err(CliError::system(
                "install_failed",
                format!("could not inspect existing `{}`: {e}", path.display()),
            ));
        }
    };

    if let Some(result) = check_non_regular(path, &metadata, force)? {
        return Ok(result);
    }

    let existing = match std::fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData && force => {
            return Ok(Preflight {
                action: PlannedAction::Overwrite,
                warning: Some(format!(
                    "overwriting non-text destination `{}` (--force)",
                    path.display()
                )),
            });
        }
        Err(e) => {
            return Err(CliError::system(
                "install_failed",
                format!("could not inspect existing `{}`: {e}", path.display()),
            ));
        }
    };
    let on_disk_name = frontmatter_field(&existing, "name");
    let on_disk_version = frontmatter_cli_version(&existing);

    if on_disk_name.as_deref() != Some(skill.name) || on_disk_version.is_none() {
        if force {
            return Ok(Preflight {
                action: PlannedAction::Overwrite,
                warning: Some(format!(
                    "overwriting unmanaged or malformed destination `{}` (--force)",
                    path.display()
                )),
            });
        }
        return Err(CliError::user(
            "skill_install_conflict",
            format!(
                "`{}` is not a managed `{}` skill; pass --force to overwrite",
                path.display(),
                skill.name
            ),
        ));
    }

    let on_disk = on_disk_version.expect("checked above");
    match compare_versions(&on_disk, CLI_VERSION) {
        Some(std::cmp::Ordering::Less) => Ok(Preflight {
            action: PlannedAction::Upgrade,
            warning: Some(format!(
                "upgrading `{}` from cli_version {on_disk} to {CLI_VERSION}",
                path.display()
            )),
        }),
        Some(std::cmp::Ordering::Equal) => Ok(Preflight {
            action: PlannedAction::Unchanged,
            warning: None,
        }),
        Some(std::cmp::Ordering::Greater) if force => Ok(Preflight {
            action: PlannedAction::Overwrite,
            warning: Some(format!(
                "downgrading `{}` from cli_version {on_disk} to {CLI_VERSION} (--force)",
                path.display()
            )),
        }),
        Some(std::cmp::Ordering::Greater) => Err(CliError::user(
            "skill_version_mismatch",
            format!(
                "`{}` has cli_version {on_disk}, newer than this binary ({CLI_VERSION}); pass --force to overwrite",
                path.display()
            ),
        )),
        None if force => Ok(Preflight {
            action: PlannedAction::Overwrite,
            warning: Some(format!(
                "overwriting `{}`: existing cli_version `{on_disk}` is unparseable (--force)",
                path.display()
            )),
        }),
        None => Err(CliError::user(
            "skill_install_conflict",
            format!(
                "`{}` has an unparseable cli_version `{on_disk}`; pass --force to overwrite",
                path.display()
            ),
        )),
    }
}

/// Never follow a final-component symlink. With `--force` the atomic rename
/// replaces it; directories cannot be replaced safely by this file installer.
fn check_non_regular(
    path: &Path,
    metadata: &std::fs::Metadata,
    force: bool,
) -> Result<Option<Preflight>, CliError> {
    if metadata.file_type().is_file() {
        return Ok(None);
    }
    if metadata.file_type().is_symlink() && force {
        return Ok(Some(Preflight {
            action: PlannedAction::Overwrite,
            warning: Some(format!(
                "overwriting non-regular destination `{}` (--force)",
                path.display()
            )),
        }));
    }
    let guidance = if metadata.file_type().is_symlink() {
        "; pass --force to replace the symlink"
    } else {
        "; expected a regular file"
    };
    Err(CliError::user(
        "skill_install_conflict",
        format!(
            "refusing to overwrite non-regular destination `{}`{guidance}",
            path.display()
        ),
    ))
}

/// Extract the top-level `cli_version:` scalar from a SKILL.md YAML frontmatter
/// block. Not a full YAML parser (the frontmatter shape is fixed and simple),
/// but deliberately strict about the cases that would otherwise misread a value:
/// a leading UTF-8 BOM is tolerated, only the leading `---`…`---` block is
/// scanned, only *top-level* keys count (an indented `cli_version:` inside a
/// nested mapping is ignored), and a trailing `# comment` and surrounding quotes
/// are stripped.
pub(crate) fn frontmatter_cli_version(text: &str) -> Option<String> {
    frontmatter_field(text, "cli_version")
}

/// Read one top-level `key:` scalar from the leading YAML frontmatter block.
pub(crate) fn frontmatter_field(text: &str, key: &str) -> Option<String> {
    // Tolerate a UTF-8 BOM some editors prepend.
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let body = text.strip_prefix("---")?;
    // The block ends at the next line that begins with `---` (require the newline
    // so an unterminated block yields `None` rather than slurping prose).
    let end = body.find("\n---")?;
    for line in body[..end].lines() {
        // Top-level keys only: an indented line is inside a nested mapping.
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(rest) = line.trim_end().strip_prefix(&format!("{key}:")) else {
            continue;
        };
        // Drop a trailing ` # comment`, then surrounding quotes.
        let v = rest.split(" #").next().unwrap_or(rest).trim();
        let v = v.trim_matches(|c| c == '"' || c == '\'');
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

/// Compare two `SemVer` `cli_version` strings. `None` if either fails to parse —
/// the caller then treats the comparison as a refusable conflict rather than
/// guessing. Uses the `semver` crate so prerelease precedence (`1.0.0-rc.1 <
/// 1.0.0`) and build metadata are handled correctly, and so the binary's own
/// `CARGO_PKG_VERSION` (which may be a prerelease) always compares equal to a
/// freshly-installed copy of itself — the idempotency §17 relies on.
fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let a = semver::Version::parse(a).ok()?;
    let b = semver::Version::parse(b).ok()?;
    Some(a.cmp(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn the_binarys_own_version_parses() {
        // §17 idempotency hinges on this: a freshly-installed copy pins
        // CLI_VERSION, and reinstalling must read it back as Equal — including
        // when CARGO_PKG_VERSION is a prerelease like `0.2.0-rc.1`.
        assert_eq!(
            compare_versions(CLI_VERSION, CLI_VERSION),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn semver_precedence_is_respected() {
        assert_eq!(compare_versions("0.1.0", "0.2.0"), Some(Ordering::Less));
        assert_eq!(
            compare_versions("1.0.0-rc.1", "1.0.0"),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_versions("0.1.0-rc.1", "0.1.0-rc.1"),
            Some(Ordering::Equal)
        );
        // Build metadata parses — it is NOT treated as unparseable (the old
        // numeric-only comparator would have refused it, breaking reinstall).
        assert!(compare_versions("0.1.0+build.7", "0.1.0").is_some());
        assert_eq!(compare_versions("not-a-version", "0.1.0"), None);
    }

    #[test]
    fn frontmatter_reads_top_level_quoted_value() {
        let text = "---\nname: x\ncli_version: \"0.3.1\"\n---\nbody\n";
        assert_eq!(frontmatter_cli_version(text).as_deref(), Some("0.3.1"));
    }

    #[test]
    fn frontmatter_tolerates_bom_and_unquoted() {
        let text = "\u{FEFF}---\ncli_version: 0.3.1\n---\n";
        assert_eq!(frontmatter_cli_version(text).as_deref(), Some("0.3.1"));
    }

    #[test]
    fn frontmatter_strips_trailing_comment() {
        let text = "---\ncli_version: \"0.3.1\" # pinned\n---\n";
        assert_eq!(frontmatter_cli_version(text).as_deref(), Some("0.3.1"));
    }

    #[test]
    fn frontmatter_ignores_nested_key() {
        // An indented `cli_version:` is inside a nested mapping — not the
        // top-level field, and must not be mistaken for it.
        let text = "---\nmetadata:\n  cli_version: \"9.9.9\"\n---\n";
        assert_eq!(frontmatter_cli_version(text), None);
    }

    #[test]
    fn frontmatter_requires_a_closing_delimiter() {
        // No closing `---`: refuse to slurp an incidental `cli_version:` from
        // the body prose.
        let text = "---\nname: x\nsome prose mentioning cli_version: fake\n";
        assert_eq!(frontmatter_cli_version(text), None);
    }

    #[test]
    fn legacy_skill_name_has_an_actionable_rename_refusal() {
        let error = lookup("oss-release").expect_err("old name must not remain an alias");
        assert_eq!(error.code, "skill_renamed");
        assert!(error.message.contains("shipshape-release"));
        assert!(error.message.contains("shipshape skill install"));
    }
}
