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
        name: "oss-architecture",
        description:
            "Opt-in architecture docs of the /oss-* family: emit a matklad-style \
             ARCHITECTURE.md code map, scaffold an ADR log + docs site, act on the `docs_site` \
             config field. Never a readiness gate; ADRs come from /worktree-technical-decision.",
        template: include_str!("../skills/oss-architecture/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-architecture/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-init",
        description:
            "Generator of a project's OSS-RELEASE.md config: read repo facts, infer the dials, \
             write a human-reviewable draft (a thin skill over `ossctl facts` + `contract validate`).",
        template: include_str!("../skills/oss-init/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-init/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-release",
        description:
            "Orchestrator/router of the /oss-* family: read the contract, score readiness, \
             sequence members, cut the release.",
        template: include_str!("../skills/oss-release/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-release/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-ci",
        description:
            "Generator of a repo's contribution-quality CI: read the contract, emit the \
             tier/ecosystem-tuned GitHub Actions workflow + dep-bot/pre-commit/security-lint \
             gates (a thin skill over `ossctl contract show`).",
        template: include_str!("../skills/oss-ci/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-ci/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-security-policy",
        description:
            "Threat-gated generator of SECURITY.md: detect an enumerated set of threat signals \
             from repo inspection + `ossctl facts`/`contract show`, and emit a full \
             coordinated-disclosure policy when the surface warrants, else a minimal pointer.",
        template: include_str!("../skills/oss-security-policy/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-security-policy/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-changelog",
        description:
            "Establish + maintain CHANGELOG.md (sole writer): Keep-a-Changelog skeleton, \
             marker-anchored [Unreleased] ops, and release finalize — reads changelog.mode from \
             `ossctl contract show`, compiles fragments via `issuectl changelog`.",
        template: include_str!("../skills/oss-changelog/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-changelog/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-readiness",
        description:
            "Score OSS-release readiness and turn the gap report into a prioritized action list \
             (a thin skill over `ossctl audit`).",
        template: include_str!("../skills/oss-readiness/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-readiness/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-readme",
        description:
            "Generator of a project's README.md front door + LICENSE: read the contract \
             (license/ecosystems/targets) and facts, emit a maturity-tiered slotted README and \
             an SPDX-correct LICENSE (a thin skill over `ossctl contract show` + `facts`).",
        template: include_str!("../skills/oss-readme/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-readme/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-contributing",
        description:
            "Generator of a project's contributor-onboarding docs: CONTRIBUTING.md plus \
             tier-gated code of conduct, issue forms, PR template, and governance — templated \
             emission tuned to the contract (a thin skill over `ossctl contract show`).",
        template: include_str!("../skills/oss-contributing/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-contributing/SKILL.template.md",
    },
    BundledSkill {
        name: "oss-dist",
        description:
            "Generator of Rust binary-distribution infrastructure: contract-driven cargo-dist \
             config + generated release workflow, Cargo dist profile, and Homebrew tap/secret \
             operations guidance (a thin skill over `ossctl contract show` + `dist generate`).",
        template: include_str!("../skills/oss-dist/SKILL.template.md"),
        path_in_repo: "crates/ossctl-cli/skills/oss-dist/SKILL.template.md",
    },
];

/// Which agent runtime(s) `skill install` targets. Each selects a well-known
/// skills directory under `$HOME` *and* the on-disk file shape that runtime
/// expects; `--dest` overrides the directory (not the shape).
///
/// When `--agent` is **omitted**, install **dual-homes** into Claude Code *and*
/// pi.dev (see [`selected_runtimes`]) — the migration default. An explicit value
/// narrows to one runtime, or, with `all`, widens to every known runtime.
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

/// The runtimes an `--agent` selection expands to. When **unset** (the common
/// case), install **dual-home** into Claude Code *and* pi.dev — the migration
/// default: an agent stack moving from Claude Code to pi.dev needs each skill
/// discoverable under both `~/.claude/skills` and `~/.pi/agent/skills`. An
/// explicit `--agent` narrows to a single runtime (or, with `all`, widens to
/// every known runtime, Codex included).
fn selected_runtimes(agent: Option<Agent>) -> &'static [Runtime] {
    match agent {
        None => &[Runtime::Claude, Runtime::Pi],
        Some(a) => a.runtimes(),
    }
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
    /// Agent runtime to install into. Omit to **dual-home** into Claude Code and
    /// pi.dev (`~/.claude/skills` + `~/.pi/agent/skills`); pass `claude`, `pi`,
    /// or `codex` to narrow to one, or `all` for every known runtime.
    #[arg(long, value_enum)]
    pub agent: Option<Agent>,
    /// Override the destination directory (the runtime's file shape still applies
    /// under it). When several selected runtimes share a shape *and* a `--dest`
    /// root — Claude and pi.dev both write `<name>/SKILL.md` — they resolve to the
    /// same file and collapse to a single write.
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
                crate::output::stdoutln!("no skills bundled");
            } else {
                for s in &skills {
                    crate::output::stdoutln!(
                        "{}  (cli {})  {}",
                        s.name,
                        s.cli_version,
                        s.description
                    );
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
        OutputFormat::Text => crate::output::stdout!("{content}"),
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
    // Which skills: the named one, or the whole catalog.
    let targets: Vec<&BundledSkill> = match &args.name {
        Some(name) => vec![lookup(name)?],
        None => CATALOG.iter().collect(),
    };

    // Resolve $HOME only when needed — a `--dest` override skips it entirely.
    let home = home_dir();

    // Build the full plan before touching disk.
    let mut plan = Vec::new();
    for &runtime in selected_runtimes(args.agent) {
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
    // the whole command *before* any partial write. Distinct *logical* targets can
    // resolve to one *physical* file — shape-sharing runtimes (Claude + pi.dev,
    // both `<name>/SKILL.md`) rooted at the same `--dest` — so check each unique
    // path once, both to avoid a duplicate warning and because the second "check"
    // would race the first's write. (Lexical dedup only: `--dest a/../a` and
    // symlinked roots are not canonicalized, but a missed collision only costs a
    // redundant write of byte-identical content, which is harmless.)
    let mut warnings = Vec::new();
    let mut checked = std::collections::HashSet::new();
    for entry in &plan {
        if checked.insert(entry.path.clone()) {
            if let Some(w) = check_drift(&entry.path, args.force)? {
                warnings.push(w);
            }
        }
    }

    // All clear — write each unique file once, but report **every** logical target.
    // A user who asked for `--agent all` still sees a `pi` row even when its file
    // coincides with Claude's under a shared `--dest`: the write collapses, the
    // reporting does not (so automation greping `agent == "pi"` still matches).
    let mut installed = Vec::new();
    let mut written = std::collections::HashSet::new();
    for (idx, entry) in plan.iter().enumerate() {
        if written.insert(entry.path.clone()) {
            let content = render(entry.skill);
            write_atomic(&entry.path, &content, idx)?;
        }
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
                crate::output::stdoutln!("installed {} ({}) → {}", e.name, e.agent, e.dest_path);
            }
        }
    }
    Ok(())
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
    let tmp = parent.join(format!(".ossctl-skill.{}.{idx}.tmp", std::process::id()));
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
        // Only a genuine "not found" is a clean first install. Every other read
        // failure (permission denied, path is a directory, invalid UTF-8) must
        // surface — treating it as "absent" would silently overwrite whatever is
        // there, defeating the drift guard.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(CliError::system(
                "install_failed",
                format!("could not inspect existing `{}`: {e}", path.display()),
            ));
        }
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
}
