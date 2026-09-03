//! Deterministic public-front-door checks learned from the project-canon and
//! Glasspad publicize passes. Judgment and mutation remain in the thin skill.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::contract::schema::{Contract, Ecosystem};
use crate::ports::{CommandRunner, Fs};
use crate::protocol::audit::{Category, Gap, Presence, Severity};
use crate::protocol::facts::Facts;

use super::{github_slug, HEALTH_DIRS, README_NAMES, SECURITY_NAMES};

/// Deterministic checks justified by the project-canon and Glasspad publicize
/// passes. Every finding is advisory: these checks describe the public front
/// door, not the gated release core. Remote failures remain `unknown`.
pub(super) fn publicize_gaps(
    gaps: &mut Vec<Gap>,
    repo_root: &Path,
    contract: &Contract,
    facts: &Facts,
    fs: &dyn Fs,
    cmd: &dyn CommandRunner,
) {
    let markdown = public_markdown(repo_root, fs);
    let readme = read_first(fs, repo_root, README_NAMES);

    if let Some(slug) = github_slug(repo_root, cmd) {
        github_metadata_gaps(gaps, repo_root, cmd, &slug);
        pvr_gap(gaps, repo_root, fs, cmd, &slug);
    }

    if let Some((_, text)) = &readme {
        readme_platform_gaps(gaps, contract, text);
        readme_command_gaps(gaps, repo_root, facts, fs, cmd, text);
    }
    symlink_link_gaps(gaps, repo_root, cmd, &markdown);
    neutrality_gaps(gaps, &markdown);
}

fn github_metadata_gaps(
    gaps: &mut Vec<Gap>,
    repo_root: &Path,
    cmd: &dyn CommandRunner,
    slug: &str,
) {
    let path = format!("repos/{slug}");
    let output = cmd.run("gh", &["api", &path], repo_root);
    let parsed = output.ok().and_then(|out| {
        (out.status == Some(0))
            .then(|| serde_json::from_str::<serde_json::Value>(&out.stdout).ok())
            .flatten()
    });
    let Some(object) = parsed.and_then(|v| v.as_object().cloned()) else {
        for (id, label) in [
            ("github-description", "description"),
            ("github-topics", "topics"),
        ] {
            gaps.push(publicize_gap(
                id,
                Presence::Unknown,
                format!("could not verify the GitHub repository {label}"),
            ));
        }
        return;
    };
    let description_present = object
        .get("description")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    if !description_present {
        gaps.push(publicize_gap(
            "github-description",
            Presence::Absent,
            "GitHub repository description is empty — the public listing has no value proposition",
        ));
    }
    let topics_present = object
        .get("topics")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|values| !values.is_empty());
    if !topics_present {
        gaps.push(publicize_gap(
            "github-topics",
            Presence::Absent,
            "GitHub repository topics are empty — the project is harder to discover",
        ));
    }
}

fn pvr_gap(
    gaps: &mut Vec<Gap>,
    repo_root: &Path,
    fs: &dyn Fs,
    cmd: &dyn CommandRunner,
    slug: &str,
) {
    let Some((_, security)) = read_first(fs, repo_root, SECURITY_NAMES) else {
        return;
    };
    let lower = security.to_lowercase();
    let references_pvr = [
        "private vulnerability reporting",
        "report a vulnerability",
        "privately-reporting-a-security-vulnerability",
    ]
    .iter()
    .any(|token| lower.contains(token));
    if !references_pvr {
        return;
    }

    let path = format!("repos/{slug}/private-vulnerability-reporting");
    let status = match cmd.run("gh", &["api", &path], repo_root) {
        Ok(out) if out.status == Some(0) => serde_json::from_str::<serde_json::Value>(&out.stdout)
            .ok()
            .and_then(|value| value.get("enabled").and_then(serde_json::Value::as_bool))
            .map_or(Presence::Unknown, |enabled| {
                if enabled {
                    Presence::Present
                } else {
                    Presence::Absent
                }
            }),
        _ => Presence::Unknown,
    };
    if status != Presence::Present {
        gaps.push(publicize_gap(
            "github-private-vulnerability-reporting",
            status,
            "SECURITY policy directs reporters to GitHub Private Vulnerability Reporting, but that repository setting is not verified enabled",
        ));
    }
}

fn readme_platform_gaps(gaps: &mut Vec<Gap>, contract: &Contract, readme: &str) {
    let declared: HashSet<&str> = contract
        .distributions
        .iter()
        .flat_map(|distribution| distribution.platforms.iter().map(String::as_str))
        .collect();
    if declared.is_empty() {
        return;
    }
    let claims = [
        (
            "aarch64-apple-darwin",
            ["aarch64-apple-darwin", "macos arm64"],
        ),
        (
            "x86_64-apple-darwin",
            ["x86_64-apple-darwin", "intel macos"],
        ),
        (
            "aarch64-unknown-linux-musl",
            ["aarch64-unknown-linux-musl", "linux arm64"],
        ),
        (
            "x86_64-unknown-linux-musl",
            ["x86_64-unknown-linux-musl", "linux x86_64"],
        ),
    ];
    let mut seen = HashSet::new();
    for line in readme.lines() {
        let lower = line.to_lowercase();
        let negative = [
            "unsupported",
            "not supported",
            "no prebuilt",
            "does not ship",
        ]
        .iter()
        .any(|token| lower.contains(token));
        if negative {
            continue;
        }
        for (triple, aliases) in &claims {
            if !declared.contains(triple)
                && aliases.iter().any(|alias| lower.contains(alias))
                && seen.insert(*triple)
            {
                gaps.push(publicize_gap(
                    &format!("readme-platform:{triple}"),
                    Presence::Absent,
                    format!("README claims prebuilt support for `{triple}`, but no distribution declares that target"),
                ));
            }
        }
    }
}

fn readme_command_gaps(
    gaps: &mut Vec<Gap>,
    repo_root: &Path,
    facts: &Facts,
    fs: &dyn Fs,
    cmd: &dyn CommandRunner,
    readme: &str,
) {
    let binaries = project_binaries(repo_root, facts, fs);
    if binaries.is_empty() {
        return;
    }
    let commands = fenced_commands(readme, &binaries);
    let mut unavailable = HashSet::new();
    for (index, tokens) in commands.iter().enumerate() {
        let binary = &tokens[0];
        let Some(top) = structured_help(cmd, repo_root, binary, &[]) else {
            if unavailable.insert(binary.clone()) {
                gaps.push(publicize_gap(
                    &format!("readme-command-help:{binary}"),
                    Presence::Unknown,
                    format!("README examples use `{binary}`, but `{binary} --help --json` could not be inspected"),
                ));
            }
            continue;
        };
        let mut help = top;
        let mut path: Vec<String> = Vec::new();
        let mut invalid = None;
        for token in tokens
            .iter()
            .skip(1)
            .take_while(|token| !token.starts_with('-'))
        {
            let subs = help_subcommands(&help);
            if subs.is_empty() {
                break;
            }
            if !subs.iter().any(|name| name == token) {
                if help_has_positionals(&help) {
                    break;
                }
                invalid = Some(token.clone());
                break;
            }
            path.push(token.clone());
            let refs: Vec<&str> = path.iter().map(String::as_str).collect();
            let Some(next) = structured_help(cmd, repo_root, binary, &refs) else {
                invalid = Some(token.clone());
                break;
            };
            help = next;
        }
        if invalid.is_none() {
            invalid = tokens
                .iter()
                .filter_map(|token| token.strip_prefix("--"))
                .map(|flag| flag.split('=').next().unwrap_or(flag))
                .find(|flag| !help_has_flag(&help, flag))
                .map(|flag| format!("--{flag}"));
        }
        if let Some(token) = invalid {
            gaps.push(publicize_gap(
                &format!("readme-command:{}", index + 1),
                Presence::Absent,
                format!("README example references `{binary} {token}`, which is not in the command's structured help tree"),
            ));
        }
    }
}

fn structured_help(
    cmd: &dyn CommandRunner,
    repo_root: &Path,
    binary: &str,
    path: &[&str],
) -> Option<serde_json::Value> {
    let mut args = path.to_vec();
    args.extend(["--help", "--json"]);
    let out = cmd.run(binary, &args, repo_root).ok()?;
    (out.status == Some(0))
        .then(|| serde_json::from_str::<serde_json::Value>(&out.stdout).ok())
        .flatten()
}

fn help_command(value: &serde_json::Value) -> Option<&serde_json::Value> {
    let data = value.get("data")?;
    Some(data.get("command").unwrap_or(data))
}

fn help_subcommands(value: &serde_json::Value) -> Vec<String> {
    help_command(value)
        .and_then(|command| command.get("subcommands"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|sub| sub.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

fn help_has_positionals(value: &serde_json::Value) -> bool {
    help_command(value)
        .and_then(|command| command.get("args"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|args| !args.is_empty())
}

fn help_has_flag(value: &serde_json::Value, wanted: &str) -> bool {
    help_command(value)
        .and_then(|command| command.get("flags"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|flags| {
            flags
                .iter()
                .any(|flag| flag.get("long").and_then(serde_json::Value::as_str) == Some(wanted))
        })
}

fn project_binaries(repo_root: &Path, facts: &Facts, fs: &dyn Fs) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Some(text) = read_text(fs, &repo_root.join("Cargo.toml"), 1 << 20) {
        if let Ok(value) = toml::from_str::<toml::Value>(&text) {
            collect_manifest_binaries(repo_root, &value, fs, &mut names);
            if let Some(members) = value
                .get("workspace")
                .and_then(|workspace| workspace.get("members"))
                .and_then(toml::Value::as_array)
            {
                for member in members.iter().filter_map(toml::Value::as_str) {
                    // The two observed repositories use literal workspace member paths.
                    // Glob expansion remains in the skill's judgment sweep.
                    if member.contains(['*', '?', '[']) {
                        continue;
                    }
                    let member_root = repo_root.join(member);
                    if let Some(text) = read_text(fs, &member_root.join("Cargo.toml"), 1 << 20) {
                        if let Ok(value) = toml::from_str::<toml::Value>(&text) {
                            collect_manifest_binaries(&member_root, &value, fs, &mut names);
                        }
                    }
                }
            }
        }
    }
    for package in &facts.packages {
        if package.ecosystem == Ecosystem::Go {
            if let Some(name) = &package.package {
                if let Some(last) = name.rsplit('/').next() {
                    names.insert(last.to_string());
                }
            }
        }
    }
    names
}

fn collect_manifest_binaries(
    manifest_root: &Path,
    value: &toml::Value,
    fs: &dyn Fs,
    names: &mut HashSet<String>,
) {
    if let Some(bins) = value.get("bin").and_then(toml::Value::as_array) {
        for bin in bins {
            if let Some(name) = bin.get("name").and_then(toml::Value::as_str) {
                names.insert(name.to_string());
            }
        }
    }
    if fs.is_file(&manifest_root.join("src/main.rs")) {
        if let Some(name) = value
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        {
            names.insert(name.to_string());
        }
    }
}

fn fenced_commands(readme: &str, binaries: &HashSet<String>) -> Vec<Vec<String>> {
    let mut inside = false;
    let mut commands = Vec::new();
    for line in readme.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if !inside {
            continue;
        }
        let line = line.trim().strip_prefix("$ ").unwrap_or(line.trim());
        let tokens: Vec<String> = line
            .split_ascii_whitespace()
            .take_while(|token| !matches!(*token, "|" | "&&" | "||"))
            .map(|token| {
                token
                    .trim_matches(|c: char| matches!(c, '`' | '\'' | '"'))
                    .to_string()
            })
            .collect();
        if tokens
            .first()
            .is_some_and(|binary| binaries.contains(binary))
        {
            commands.push(tokens);
        }
    }
    commands
}

fn symlink_link_gaps(
    gaps: &mut Vec<Gap>,
    repo_root: &Path,
    cmd: &dyn CommandRunner,
    markdown: &[(PathBuf, String)],
) {
    let Ok(out) = cmd.run("git", &["ls-files", "-s"], repo_root) else {
        return;
    };
    if out.status != Some(0) {
        return;
    }
    let symlinks: HashSet<PathBuf> = out
        .stdout
        .lines()
        .filter_map(|line| {
            let (metadata, path) = line.split_once('\t')?;
            metadata.starts_with("120000 ").then(|| PathBuf::from(path))
        })
        .collect();
    if symlinks.is_empty() {
        return;
    }
    let mut seen = HashSet::new();
    for (source, text) in markdown {
        for destination in markdown_destinations(text) {
            if destination.starts_with('#') || destination.contains("://") {
                continue;
            }
            let destination = destination.split(['#', '?']).next().unwrap_or(&destination);
            let parent = source.parent().unwrap_or_else(|| Path::new(""));
            let target = normalize_relative(parent.join(destination));
            if symlinks.contains(&target) && seen.insert((source.clone(), target.clone())) {
                gaps.push(publicize_gap(
                    &format!("symlink-link:{}:{}", source.display(), target.display()),
                    Presence::Absent,
                    format!("public document `{}` links to tracked symlink `{}`; GitHub renders the symlink target path rather than the target content", source.display(), target.display()),
                ));
            }
        }
    }
}

fn neutrality_gaps(gaps: &mut Vec<Gap>, markdown: &[(PathBuf, String)]) {
    let phrases = ["claude code skill", "claude-code skill", "claude skill"];
    for (path, text) in markdown {
        let lower = text.to_lowercase();
        if phrases.iter().any(|phrase| lower.contains(phrase)) {
            gaps.push(publicize_gap(
                &format!("product-neutrality:{}", path.display()),
                Presence::Absent,
                format!("public document `{}` uses one agent product as the category name for skills; use the neutral Agent Skills terminology", path.display()),
            ));
        }
    }
}

fn public_markdown(repo_root: &Path, fs: &dyn Fs) -> Vec<(PathBuf, String)> {
    let mut paths: Vec<PathBuf> = [
        "README.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "ARCHITECTURE.md",
        "GOVERNANCE.md",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    collect_markdown(fs, repo_root, Path::new("docs"), &mut paths, 0);
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter_map(|path| read_text(fs, &repo_root.join(&path), 1 << 20).map(|text| (path, text)))
        .collect()
}

fn collect_markdown(
    fs: &dyn Fs,
    repo_root: &Path,
    relative: &Path,
    paths: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > 8 || matches!(relative.to_str(), Some("docs/adr" | "docs/recovery")) {
        return;
    }
    let Ok(entries) = fs.read_dir(&repo_root.join(relative)) else {
        return;
    };
    for name in entries {
        let child = relative.join(name);
        let full = repo_root.join(&child);
        if fs.is_dir(&full) {
            collect_markdown(fs, repo_root, &child, paths, depth + 1);
        } else if child.extension().and_then(|value| value.to_str()) == Some("md") {
            paths.push(child);
        }
    }
}

fn markdown_destinations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        while let Some(start) = rest.find("](") {
            let after = &rest[start + 2..];
            let Some(end) = after.find(')') else { break };
            let value = after[..end].trim().trim_matches(['<', '>']);
            if !value.is_empty() {
                out.push(value.to_string());
            }
            rest = &after[end + 1..];
        }
        if let Some((prefix, value)) = line.split_once(": ") {
            if prefix.trim_start().starts_with('[') && prefix.trim_end().ends_with(']') {
                out.push(
                    value
                        .split_ascii_whitespace()
                        .next()
                        .unwrap_or(value)
                        .to_string(),
                );
            }
        }
    }
    out
}

fn normalize_relative(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(value) => out.push(value),
            Component::RootDir | Component::Prefix(_) => return path,
        }
    }
    out
}

fn read_first(fs: &dyn Fs, repo_root: &Path, names: &[&str]) -> Option<(PathBuf, String)> {
    names.iter().find_map(|name| {
        HEALTH_DIRS.iter().find_map(|dir| {
            let relative = if dir.is_empty() {
                PathBuf::from(name)
            } else {
                PathBuf::from(dir).join(name)
            };
            read_text(fs, &repo_root.join(&relative), 1 << 20).map(|text| (relative, text))
        })
    })
}

fn read_text(fs: &dyn Fs, path: &Path, limit: usize) -> Option<String> {
    let bytes = fs.read(path).ok()?;
    let bytes = &bytes[..bytes.len().min(limit)];
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn publicize_gap(id: &str, status: Presence, detail: impl Into<String>) -> Gap {
    Gap {
        id: id.to_string(),
        category: Category::Canon,
        severity: Severity::Recommended,
        status,
        member: "shipshape-publicize".to_string(),
        detail: detail.into(),
    }
}
