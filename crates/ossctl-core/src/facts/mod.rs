//! Deterministic repo-fact detector (port of `infer-repo-facts.py` —
//! ADR-0001 §3).
//!
//! [`gather`] is a pure function of `(repo tree, git HEAD)`: it sniffs
//! ecosystems and manifests, counts committers, reads tags, detects CI/bot/
//! issues signals, extracts a README self-label and description, and applies the
//! SCHEMA.md §4 maturity truth table. All I/O goes through the [`Fs`] and
//! [`GitRepo`] ports (`crate::ports`), so the detector is exercised entirely
//! against in-memory fakes and never touches the real filesystem or git — the
//! whole point of the injected-port seam (ADR-0001 §2).
//!
//! The report shape lives in [`crate::protocol::facts`] (the versioned wire
//! DTO); this module owns only the detection logic. It is **reproducible**: for
//! a fixed repository state *and* wall-clock day it produces byte-identical
//! facts, so `/oss-init` and the readiness `audit` reading the same `ossctl
//! facts` output agree on maturity and the gated core. It is not a pure function
//! of `HEAD` alone: the recent-committer count uses git's `--since=1 year ago`
//! (evaluated against the current clock) and reads all refs/tags, so a run that
//! crosses the one-year boundary, or after refs change, can shift — the same
//! time-relative behavior `infer-repo-facts.py` has.
//!
//! ## Fidelity to the Python detector
//!
//! Field names, the manifest set, the SemVer/monorepo-prefix handling, the CI
//! globs, the spike-label list, and the maturity truth table all mirror
//! `infer-repo-facts.py`. TOML manifests (`Cargo.toml`, `pyproject.toml`) are
//! parsed by scanning the relevant `[section]` for `key = "value"` (single or
//! double quotes) rather than via a full TOML library — the contract normalizer
//! avoids new parser deps the same way. This matches the Python `tomllib` path
//! for the common manifest shapes; two edge cases are **not** reproduced and are
//! deliberately out of scope: escaped/multiline TOML strings, and Python's
//! whole-file regex *fallback* that fires only when `tomllib` itself fails on
//! malformed TOML (there, Python may surface a `name` from an unrelated table;
//! this port returns none). Both are exotic in a real `pyproject.toml`.

use std::path::Path;

use crate::contract::schema::{Ecosystem, Maturity};
use crate::ports::{Fs, GitRepo};
use crate::protocol::facts::{Facts, MaturitySignals, Package};

/// Root-level manifests, in the canonical probe order. The ecosystem order here
/// also fixes the order `ecosystems` and `packages` are emitted in.
const MANIFESTS: &[(&str, Ecosystem)] = &[
    ("Cargo.toml", Ecosystem::Rust),
    ("package.json", Ecosystem::Node),
    ("pyproject.toml", Ecosystem::Python),
    ("setup.py", Ecosystem::Python),
    ("go.mod", Ecosystem::Go),
];

/// README tokens that self-label a project as pre-release (a spike signal).
/// Matched case-insensitively as whole substrings; kept small + explicit for
/// reproducibility (mirrors the Python `SPIKE_LABELS`).
const SPIKE_LABELS: &[&str] = &[
    "work in progress",
    "work-in-progress",
    "wip",
    "experimental",
    "prototype",
    "pre-alpha",
    "proof of concept",
    "proof-of-concept",
    "status: private, early",
    "not production ready",
    "not production-ready",
    "early prototype",
    "spike",
];

/// CI configuration paths. `.github/workflows` is a directory that counts only
/// when non-empty; the rest are single-file configs that count on existence.
const CI_GLOBS: &[&str] = &[
    ".github/workflows",
    ".gitlab-ci.yml",
    ".circleci",
    "azure-pipelines.yml",
    ".drone.yml",
    "Jenkinsfile",
];

/// Character cap for a manifest read (matches the Python `_read` default, which
/// reads decoded characters, not bytes).
const MANIFEST_LIMIT: usize = 200_000;
/// Character cap for a README read (matches the Python text-mode README read).
const README_LIMIT: usize = 4_000;
/// Character cap for the emitted `description`.
const DESCRIPTION_CHARS: usize = 120;

/// Detect the deterministic repo facts under `repo_root`.
///
/// `repo_root` is used verbatim for the emitted `repo_root` field and as the
/// join base for filesystem probes; the caller canonicalizes it (mirroring the
/// Python `os.path.realpath`) before passing it in. Never mutates anything.
#[must_use]
pub fn gather(repo_root: &Path, fs: &dyn Fs, git: &dyn GitRepo) -> Facts {
    let is_git = git.is_work_tree();
    // `has_commits` needs both a work tree and a resolvable HEAD (an unborn repo
    // is a work tree with no HEAD).
    let has_commits = is_git && git.head_commit().is_ok();

    let (ecosystems, packages) = detect_manifests(repo_root, fs);

    // ── committers (mailmap-aware, whole `--all` history) ──
    let (committers_total, committers_recent_year) = if has_commits {
        let total = git.shortlog(None).map_or(0, count_lines);
        let recent = git.shortlog(Some("1 year ago")).map_or(0, count_lines);
        (total, recent)
    } else {
        (0, 0)
    };

    // ── tags / releases ──
    let tags = if has_commits {
        git.tags().unwrap_or_default()
    } else {
        Vec::new()
    };
    let semver_tags: Vec<(u64, u64, u64, bool)> =
        tags.iter().filter_map(|t| semver_parse(t)).collect();
    let has_semver_tag = !semver_tags.is_empty();
    let ge_1_0_tag = semver_tags
        .iter()
        .any(|&(major, _, _, pre)| major >= 1 && !pre);
    let manifest_ge_1_0 = packages
        .iter()
        .any(|p| version_ge_1_0(p.version.as_deref()));
    let has_ge_1_0_release = ge_1_0_tag || manifest_ge_1_0;

    // ── CI presence ──
    let has_ci = CI_GLOBS.iter().any(|glob| {
        let path = repo_root.join(glob);
        if !fs.exists(&path) {
            return false;
        }
        if glob.ends_with("workflows") {
            // A workflows *directory* counts only when it holds an entry.
            fs.read_dir(&path).is_ok_and(|e| !e.is_empty())
        } else {
            true
        }
    });

    // ── dependency bot ──
    let dependency_bot = if fs.is_file(&repo_root.join(".github/dependabot.yml")) {
        Some("dependabot".to_string())
    } else if ["renovate.json", ".renovaterc", ".renovaterc.json"]
        .iter()
        .any(|f| fs.is_file(&repo_root.join(f)))
    {
        Some("renovate".to_string())
    } else {
        None
    };

    let has_issues_dir = fs.is_dir(&repo_root.join("issues"));

    // ── README self-label + description ──
    let readme_text = ["README.md", "README.rst", "README.txt", "README"]
        .iter()
        .find_map(|name| {
            let text = read_text(fs, &repo_root.join(name), README_LIMIT)?;
            (!text.is_empty()).then_some(text)
        });
    let readme_self_label = readme_text.as_deref().and_then(|text| {
        let low = text.to_lowercase();
        SPIKE_LABELS
            .iter()
            .any(|label| low.contains(label))
            .then(|| "spike".to_string())
    });
    let description = detect_description(repo_root, fs, &packages, readme_text.as_deref());

    // ── maturity inference (SCHEMA.md §4, production first, tie → mvp) ──
    let production = committers_recent_year >= 2 && has_ge_1_0_release && has_ci;
    let spike =
        !has_ci && !has_semver_tag && (committers_total <= 1 || readme_self_label.is_some());
    let inferred_maturity = if production {
        Maturity::Production
    } else if spike {
        Maturity::Spike
    } else {
        Maturity::Mvp
    };

    Facts {
        repo_root: repo_root.display().to_string(),
        is_git,
        has_commits,
        ecosystems,
        packages,
        committers_total,
        committers_recent_year,
        tags,
        has_semver_tag,
        has_ge_1_0_release,
        has_ci,
        dependency_bot,
        has_issues_dir,
        readme_self_label,
        description,
        maturity_signals: MaturitySignals { production, spike },
        inferred_maturity,
    }
}

/// Sniff root-level manifests into the ordered `ecosystems` + `packages` lists.
fn detect_manifests(repo_root: &Path, fs: &dyn Fs) -> (Vec<Ecosystem>, Vec<Package>) {
    let mut ecosystems: Vec<Ecosystem> = Vec::new();
    let mut packages: Vec<Package> = Vec::new();
    for &(fname, eco) in MANIFESTS {
        let path = repo_root.join(fname);
        if !fs.is_file(&path) {
            continue;
        }
        let parsed = read_text(fs, &path, MANIFEST_LIMIT)
            .and_then(|text| parse_manifest(fname, &text))
            .unwrap_or_default();
        // A Cargo virtual workspace (no `[package]`) still marks the repo rust.
        if !ecosystems.contains(&eco) {
            ecosystems.push(eco);
        }
        // `Cargo.toml`/`go.mod` always yield a package entry (even without a
        // declared name); the others only when they name a package.
        if parsed.package.is_some() || fname == "Cargo.toml" || fname == "go.mod" {
            packages.push(Package {
                ecosystem: eco,
                manifest: fname.to_string(),
                package: parsed.package,
                version: parsed.version,
            });
        }
    }
    // `binary` only when NO package ecosystem is detected — never additive.
    if ecosystems.is_empty() {
        ecosystems.push(Ecosystem::Binary);
    }
    (ecosystems, packages)
}

/// The description: first non-empty manifest `description`, else the first
/// non-heading README line — both trimmed and truncated to 120 characters.
fn detect_description(
    repo_root: &Path,
    fs: &dyn Fs,
    packages: &[Package],
    readme_text: Option<&str>,
) -> Option<String> {
    let manifest_desc = packages.iter().find_map(|p| {
        let text = read_text(fs, &repo_root.join(&p.manifest), MANIFEST_LIMIT)?;
        let desc = parse_manifest(&p.manifest, &text)?.description?;
        (!desc.is_empty()).then_some(desc)
    });
    if let Some(desc) = manifest_desc {
        return Some(truncate_chars(desc.trim(), DESCRIPTION_CHARS));
    }
    readme_text?.lines().find_map(|line| {
        let s = line.trim();
        let is_prose =
            !s.is_empty() && !s.starts_with('#') && !s.starts_with('!') && !s.starts_with('>');
        is_prose.then(|| truncate_chars(s, DESCRIPTION_CHARS))
    })
}

// ── Manifest parsing (name + version + description) ──────────────────────────

/// The name/version/description parsed from one manifest.
#[derive(Debug, Default)]
struct ParsedManifest {
    package: Option<String>,
    version: Option<String>,
    description: Option<String>,
}

/// Dispatch to the per-manifest parser. `setup.py` yields nothing (its metadata
/// is executable, not declarative — the Python detector skips it too).
fn parse_manifest(fname: &str, text: &str) -> Option<ParsedManifest> {
    match fname {
        "Cargo.toml" => parse_cargo(text),
        "package.json" => parse_package_json(text),
        "pyproject.toml" => Some(parse_pyproject(text)),
        "go.mod" => Some(parse_gomod(text)),
        _ => None, // setup.py
    }
}

/// Parse a Cargo manifest's `[package]` block. Returns `None` for a virtual
/// workspace (no `[package]`), which still marks the repo rust upstream.
fn parse_cargo(text: &str) -> Option<ParsedManifest> {
    let block = toml_section(text, "package")?;
    Some(ParsedManifest {
        package: toml_str_value(&block, "name", false),
        version: toml_str_value(&block, "version", false),
        description: toml_str_value(&block, "description", true),
    })
}

fn parse_package_json(text: &str) -> Option<ParsedManifest> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let field = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    Some(ParsedManifest {
        package: field("name"),
        version: field("version"),
        description: field("description"),
    })
}

/// Parse a `pyproject.toml`: the standard `[project]` table first, then a legacy
/// `[tool.poetry]` fallback when `[project]` names no package.
fn parse_pyproject(text: &str) -> ParsedManifest {
    let mut parsed = ParsedManifest::default();
    if let Some(block) = toml_section(text, "project") {
        parsed.package = toml_str_value(&block, "name", false);
        parsed.version = toml_str_value(&block, "version", false);
        parsed.description = toml_str_value(&block, "description", true);
    }
    if parsed.package.is_none() {
        if let Some(block) = toml_section(text, "tool.poetry") {
            parsed.package = toml_str_value(&block, "name", false);
            parsed.version = toml_str_value(&block, "version", false);
            parsed.description = toml_str_value(&block, "description", true);
        }
    }
    parsed
}

/// Parse a `go.mod`'s `module <path>` line. Always yields a (possibly empty)
/// result — a `go.mod` marks the repo go regardless of a `module` line.
fn parse_gomod(text: &str) -> ParsedManifest {
    let module = text.lines().find_map(|line| {
        line.strip_prefix("module")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_string)
    });
    ParsedManifest {
        package: module,
        version: None,
        description: None,
    }
}

/// Extract a TOML `[header]` section body: every line after the header line up
/// to the next `[...]` line or end of file. `None` when the header is absent.
/// The header must begin the line (no indentation), matching the Python `^\[`.
fn toml_section(text: &str, header: &str) -> Option<String> {
    let needle = format!("[{header}]");
    let mut in_section = false;
    let mut out = String::new();
    for line in text.lines() {
        if in_section {
            if line.starts_with('[') {
                break;
            }
            out.push_str(line);
            out.push('\n');
        } else if line.starts_with(&needle) {
            in_section = true;
        }
    }
    in_section.then_some(out)
}

/// Find `key = "value"` within a TOML section body and return the quoted value.
/// `allow_empty` controls whether an empty `""` counts (the Python `name`/
/// `version` patterns require non-empty; `description` allows empty).
fn toml_str_value(block: &str, key: &str, allow_empty: bool) -> Option<String> {
    for line in block.lines() {
        let rest = line.trim_start();
        let Some(rest) = rest.strip_prefix(key) else {
            continue;
        };
        // The key must be a whole token: the char after it is whitespace or `=`
        // (else `name` would spuriously match `nameservers`). Mirrors the Python
        // `^\s*<key>\s*=` anchor.
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '=') {
            continue;
        }
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let Some(value) = extract_quoted(rest.trim_start()) else {
            continue;
        };
        if value.is_empty() && !allow_empty {
            return None;
        }
        return Some(value);
    }
    None
}

/// Read a leading quoted string — `"..."` or `'...'`. TOML allows both basic
/// (double) and literal (single) strings, and `tomllib` accepts either, so both
/// are honored here for parity. No escape handling: neither this nor the Python
/// regex `"([^"]+)"` unescapes, and manifest name/version/description do not need
/// it in practice.
fn extract_quoted(s: &str) -> Option<String> {
    let quote = s.chars().next().filter(|&c| c == '"' || c == '\'')?;
    let s = &s[1..];
    let end = s.find(quote)?;
    Some(s[..end].to_string())
}

// ── SemVer helpers ───────────────────────────────────────────────────────────

/// Parse a possibly package-prefixed `SemVer` tag into
/// `(major, minor, patch, is_prerelease)`, or `None` if it is not `SemVer`.
///
/// Strips a monorepo `pkg-`/`pkg@`/`pkg/` prefix (e.g. `core-v1.2.3`,
/// `@acme/cli@2.0.0`) before parsing, mirroring the Python `_semver_parse`.
fn semver_parse(tag: &str) -> Option<(u64, u64, u64, bool)> {
    parse_semver_core(strip_pkg_prefix(tag))
}

/// Strip everything up to and including the rightmost `@`/`/`/`-` that is
/// immediately followed by an optional `v` and a `X.Y.Z` version.
fn strip_pkg_prefix(tag: &str) -> &str {
    let bytes = tag.as_bytes();
    for i in (0..bytes.len()).rev() {
        if matches!(bytes[i], b'@' | b'/' | b'-') {
            let rest = &tag[i + 1..];
            if starts_with_version(rest) {
                return rest;
            }
        }
    }
    tag
}

/// Whether `s` begins with `v?\d+\.\d+\.\d+` (the version-start lookahead).
fn starts_with_version(s: &str) -> bool {
    let mut rest = s.strip_prefix('v').unwrap_or(s);
    for i in 0..3 {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return false;
        }
        rest = &rest[digits..];
        if i < 2 {
            match rest.strip_prefix('.') {
                Some(r) => rest = r,
                None => return false,
            }
        }
    }
    true
}

/// Fully parse a version core `v?\d+\.\d+\.\d+(?:[-+].*)?`. The prerelease flag
/// is set when a `-` (not `+`) immediately follows `X.Y.Z`.
fn parse_semver_core(core: &str) -> Option<(u64, u64, u64, bool)> {
    let mut rest = core.strip_prefix('v').unwrap_or(core);
    let mut nums = [0u64; 3];
    for (i, slot) in nums.iter_mut().enumerate() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        *slot = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        if i < 2 {
            rest = rest.strip_prefix('.')?;
        }
    }
    let pre = match rest.chars().next() {
        None | Some('+') => false,
        Some('-') => true,
        Some(_) => return None, // trailing junk after X.Y.Z → not SemVer
    };
    Some((nums[0], nums[1], nums[2], pre))
}

/// Whether a manifest version string is `>=1.0` (`v?\d+\.` with major `>=1`).
fn version_ge_1_0(version: Option<&str>) -> bool {
    let Some(v) = version else {
        return false;
    };
    let s = v.strip_prefix('v').unwrap_or(v);
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    // A leading number followed by a `.` — bare `1` (no dot) does not qualify.
    if digits == 0 || !s[digits..].starts_with('.') {
        return false;
    }
    s[..digits].parse::<u64>().is_ok_and(|n| n >= 1)
}

// ── Small helpers ────────────────────────────────────────────────────────────

/// Read a file through the [`Fs`] port as lossy UTF-8, capped at `limit`
/// *characters* (not bytes) — the Python `_read` opens in text mode, so
/// `fh.read(limit)` counts decoded characters. Slicing bytes instead would
/// under-read a multibyte README (a 4000-byte cap holds only ~1333 CJK chars)
/// and could split a codepoint into a `U+FFFD`. `None` when the read fails.
fn read_text(fs: &dyn Fs, path: &Path, limit: usize) -> Option<String> {
    let bytes = fs.read(path).ok()?;
    Some(
        String::from_utf8_lossy(&bytes)
            .chars()
            .take(limit)
            .collect(),
    )
}

/// Count non-blank lines (git shortlog emits one per committer).
fn count_lines(text: String) -> usize {
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// Truncate to at most `n` characters (not bytes) — the Python `[:n]` slice.
fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    // ── In-memory fakes for the ports ──────────────────────────────────────

    #[derive(Default)]
    struct FakeFs {
        files: HashMap<PathBuf, Vec<u8>>,
        dirs: HashSet<PathBuf>,
    }

    impl FakeFs {
        fn file(mut self, path: &str, contents: &str) -> Self {
            let p = PathBuf::from(path);
            // Register ancestor directories so `read_dir`/`is_dir` see them.
            let mut cur = p.parent();
            while let Some(dir) = cur {
                if dir.as_os_str().is_empty() {
                    break;
                }
                self.dirs.insert(dir.to_path_buf());
                cur = dir.parent();
            }
            self.files.insert(p, contents.as_bytes().to_vec());
            self
        }

        fn dir(mut self, path: &str) -> Self {
            self.dirs.insert(PathBuf::from(path));
            self
        }
    }

    impl Fs for FakeFs {
        fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        }
        fn exists(&self, path: &Path) -> bool {
            self.files.contains_key(path) || self.dirs.contains(path)
        }
        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.contains(path)
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.contains_key(path)
        }
        fn read_dir(&self, dir: &Path) -> std::io::Result<Vec<String>> {
            if !self.dirs.contains(dir) {
                return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
            }
            let mut names: Vec<String> = self
                .files
                .keys()
                .chain(self.dirs.iter())
                .filter(|p| p.parent() == Some(dir))
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            names.sort();
            Ok(names)
        }
    }

    #[derive(Default)]
    struct FakeGit {
        work_tree: bool,
        head: Option<String>,
        shortlog_all: String,
        shortlog_recent: String,
        tags: Vec<String>,
    }

    impl GitRepo for FakeGit {
        fn head_commit(&self) -> std::io::Result<String> {
            self.head
                .clone()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        }
        fn is_work_tree(&self) -> bool {
            self.work_tree
        }
        fn shortlog(&self, since: Option<&str>) -> std::io::Result<String> {
            if !self.work_tree {
                return Err(std::io::Error::from(std::io::ErrorKind::Other));
            }
            Ok(if since.is_some() {
                self.shortlog_recent.clone()
            } else {
                self.shortlog_all.clone()
            })
        }
        fn tags(&self) -> std::io::Result<Vec<String>> {
            if self.work_tree {
                Ok(self.tags.clone())
            } else {
                Err(std::io::Error::from(std::io::ErrorKind::Other))
            }
        }
        fn git_common_dir(&self) -> std::io::Result<PathBuf> {
            Ok(PathBuf::from("/repo/.git"))
        }
    }

    /// A git repo with `n` distinct committers total / recent and the given tags.
    fn git_with(total: usize, recent: usize, tags: &[&str]) -> FakeGit {
        let lines = |n: usize| {
            (0..n)
                .map(|i| format!("     3\tDev {i} <dev{i}@example.com>"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        FakeGit {
            work_tree: true,
            head: Some("deadbeef".to_string()),
            shortlog_all: lines(total),
            shortlog_recent: lines(recent),
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
        }
    }

    fn repo() -> &'static Path {
        Path::new("/repo")
    }

    // ── Empty / unborn repo ────────────────────────────────────────────────

    #[test]
    fn empty_repo_is_spike_binary() {
        let facts = gather(repo(), &FakeFs::default(), &FakeGit::default());
        assert!(!facts.is_git);
        assert!(!facts.has_commits);
        assert_eq!(facts.ecosystems, vec![Ecosystem::Binary]);
        assert!(facts.packages.is_empty());
        assert_eq!(facts.committers_total, 0);
        assert_eq!(facts.committers_recent_year, 0);
        assert!(facts.tags.is_empty());
        assert!(!facts.has_ci);
        assert_eq!(facts.dependency_bot, None);
        assert_eq!(facts.description, None);
        // No CI, no SemVer tag, <=1 committer → spike.
        assert!(facts.maturity_signals.spike);
        assert_eq!(facts.inferred_maturity, Maturity::Spike);
    }

    #[test]
    fn unborn_repo_has_no_commits() {
        // A work tree whose HEAD does not resolve (no commits yet): is_git true,
        // has_commits false, so no committers/tags are read.
        let git = FakeGit {
            work_tree: true,
            head: None,
            ..FakeGit::default()
        };
        let facts = gather(repo(), &FakeFs::default(), &git);
        assert!(facts.is_git);
        assert!(!facts.has_commits);
        assert_eq!(facts.committers_total, 0);
        assert!(facts.tags.is_empty());
    }

    // ── Ecosystem + manifest detection ─────────────────────────────────────

    #[test]
    fn cargo_package_name_version_description() {
        let cargo = "[package]\nname = \"rg\"\nversion = \"0.3.0\"\n\
                     description = \"a fast grep\"\n\n[dependencies]\nserde = \"1\"\n";
        let fs = FakeFs::default().file("/repo/Cargo.toml", cargo);
        let facts = gather(repo(), &fs, &FakeGit::default());
        assert_eq!(facts.ecosystems, vec![Ecosystem::Rust]);
        assert_eq!(facts.packages.len(), 1);
        let p = &facts.packages[0];
        assert_eq!(p.ecosystem, Ecosystem::Rust);
        assert_eq!(p.manifest, "Cargo.toml");
        assert_eq!(p.package.as_deref(), Some("rg"));
        assert_eq!(p.version.as_deref(), Some("0.3.0"));
        assert_eq!(facts.description.as_deref(), Some("a fast grep"));
    }

    #[test]
    fn cargo_virtual_workspace_marks_rust_with_null_package() {
        // No `[package]` — a virtual workspace still counts as rust, and
        // Cargo.toml always yields a package entry (with null name/version).
        let fs = FakeFs::default().file("/repo/Cargo.toml", "[workspace]\nmembers = [\"a\"]\n");
        let facts = gather(repo(), &fs, &FakeGit::default());
        assert_eq!(facts.ecosystems, vec![Ecosystem::Rust]);
        assert_eq!(facts.packages.len(), 1);
        assert_eq!(facts.packages[0].package, None);
        assert_eq!(facts.packages[0].version, None);
    }

    #[test]
    fn package_json_parsed_and_go_mod_module() {
        let pkg = r#"{"name": "@acme/cli", "version": "2.0.0", "description": "cli"}"#;
        let fs = FakeFs::default()
            .file("/repo/package.json", pkg)
            .file("/repo/go.mod", "module github.com/acme/tool\n\ngo 1.22\n");
        let facts = gather(repo(), &fs, &FakeGit::default());
        assert_eq!(facts.ecosystems, vec![Ecosystem::Node, Ecosystem::Go]);
        let node = facts
            .packages
            .iter()
            .find(|p| p.ecosystem == Ecosystem::Node)
            .unwrap();
        assert_eq!(node.package.as_deref(), Some("@acme/cli"));
        assert_eq!(node.version.as_deref(), Some("2.0.0"));
        let go = facts
            .packages
            .iter()
            .find(|p| p.ecosystem == Ecosystem::Go)
            .unwrap();
        assert_eq!(go.package.as_deref(), Some("github.com/acme/tool"));
        // package.json's description wins (first package with a description).
        assert_eq!(facts.description.as_deref(), Some("cli"));
    }

    #[test]
    fn pyproject_project_then_poetry_fallback() {
        let project = "[project]\nname = \"widget\"\nversion = \"1.4.0\"\n\
                       description = \"a widget\"\n";
        let facts = gather(
            repo(),
            &FakeFs::default().file("/repo/pyproject.toml", project),
            &FakeGit::default(),
        );
        assert_eq!(facts.packages[0].package.as_deref(), Some("widget"));
        assert_eq!(facts.packages[0].version.as_deref(), Some("1.4.0"));

        let poetry = "[tool.poetry]\nname = \"legacy\"\nversion = \"0.1.0\"\n";
        let facts = gather(
            repo(),
            &FakeFs::default().file("/repo/pyproject.toml", poetry),
            &FakeGit::default(),
        );
        assert_eq!(facts.packages[0].package.as_deref(), Some("legacy"));
    }

    #[test]
    fn pyproject_single_quoted_strings_parse() {
        // TOML literal (single-quoted) strings are valid and `tomllib` accepts
        // them; the scanner must too, or a >=1.0 release would be missed.
        let project = "[project]\nname = 'widget'\nversion = '1.2.0'\n\
                       description = 'a widget'\n";
        let facts = gather(
            repo(),
            &FakeFs::default().file("/repo/pyproject.toml", project),
            &FakeGit::default(),
        );
        assert_eq!(facts.packages[0].package.as_deref(), Some("widget"));
        assert_eq!(facts.packages[0].version.as_deref(), Some("1.2.0"));
        assert!(facts.has_ge_1_0_release);
        assert_eq!(facts.description.as_deref(), Some("a widget"));
    }

    #[test]
    fn toml_key_matches_whole_token_not_prefix() {
        // `version-code` / `namespace` must not satisfy the `version` / `name`
        // key match.
        let cargo = "[package]\nnamespace = \"nope\"\nversion-code = \"9\"\n\
                     name = \"real\"\nversion = \"0.2.0\"\n";
        let facts = gather(
            repo(),
            &FakeFs::default().file("/repo/Cargo.toml", cargo),
            &FakeGit::default(),
        );
        assert_eq!(facts.packages[0].package.as_deref(), Some("real"));
        assert_eq!(facts.packages[0].version.as_deref(), Some("0.2.0"));
    }

    #[test]
    fn setup_py_marks_python_but_adds_no_package() {
        let fs = FakeFs::default().file("/repo/setup.py", "from setuptools import setup\n");
        let facts = gather(repo(), &fs, &FakeGit::default());
        assert_eq!(facts.ecosystems, vec![Ecosystem::Python]);
        assert!(facts.packages.is_empty());
    }

    #[test]
    fn ecosystems_emit_in_canonical_order() {
        // Files added out of order; output follows the MANIFESTS order.
        let fs = FakeFs::default().file("/repo/go.mod", "module x\n").file(
            "/repo/Cargo.toml",
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
        );
        let facts = gather(repo(), &fs, &FakeGit::default());
        assert_eq!(facts.ecosystems, vec![Ecosystem::Rust, Ecosystem::Go]);
    }

    // ── CI / bot / issues signals ──────────────────────────────────────────

    #[test]
    fn workflows_dir_counts_only_when_non_empty() {
        // Empty workflows dir → no CI.
        let empty = FakeFs::default().dir("/repo/.github/workflows").file(
            "/repo/Cargo.toml",
            "[package]\nname=\"a\"\nversion=\"0.1.0\"\n",
        );
        assert!(!gather(repo(), &empty, &FakeGit::default()).has_ci);

        // A file inside it → CI present.
        let with_wf = FakeFs::default()
            .file("/repo/.github/workflows/ci.yml", "on: push\n")
            .file(
                "/repo/Cargo.toml",
                "[package]\nname=\"a\"\nversion=\"0.1.0\"\n",
            );
        assert!(gather(repo(), &with_wf, &FakeGit::default()).has_ci);
    }

    #[test]
    fn single_file_ci_configs_count_on_existence() {
        for name in [
            ".gitlab-ci.yml",
            "azure-pipelines.yml",
            ".drone.yml",
            "Jenkinsfile",
        ] {
            let fs = FakeFs::default().file(&format!("/repo/{name}"), "ci\n");
            assert!(
                gather(repo(), &fs, &FakeGit::default()).has_ci,
                "{name} should count as CI"
            );
        }
    }

    #[test]
    fn dependency_bot_and_issues_dir() {
        let dependabot = FakeFs::default().file("/repo/.github/dependabot.yml", "version: 2\n");
        assert_eq!(
            gather(repo(), &dependabot, &FakeGit::default()).dependency_bot,
            Some("dependabot".to_string())
        );
        let renovate = FakeFs::default().file("/repo/renovate.json", "{}\n");
        assert_eq!(
            gather(repo(), &renovate, &FakeGit::default()).dependency_bot,
            Some("renovate".to_string())
        );
        // dependabot takes precedence when both are present.
        let both = FakeFs::default()
            .file("/repo/.github/dependabot.yml", "version: 2\n")
            .file("/repo/renovate.json", "{}\n");
        assert_eq!(
            gather(repo(), &both, &FakeGit::default()).dependency_bot,
            Some("dependabot".to_string())
        );
        let issues = FakeFs::default().dir("/repo/issues");
        assert!(gather(repo(), &issues, &FakeGit::default()).has_issues_dir);
    }

    // ── README self-label + description fallback ───────────────────────────

    #[test]
    fn readme_self_label_and_prose_description() {
        let readme = "# My Tool\n\n> a quote\n\nStatus: private, early. Not much yet.\n";
        let fs = FakeFs::default().file("/repo/README.md", readme);
        let facts = gather(repo(), &fs, &FakeGit::default());
        assert_eq!(facts.readme_self_label.as_deref(), Some("spike"));
        // First non-heading, non-`!`, non-`>` line.
        assert_eq!(
            facts.description.as_deref(),
            Some("Status: private, early. Not much yet.")
        );
    }

    #[test]
    fn description_truncates_to_120_chars() {
        let long = "x".repeat(200);
        let fs = FakeFs::default().file("/repo/README.md", &format!("intro\n{long}\n"));
        let facts = gather(repo(), &fs, &FakeGit::default());
        // "intro" is the first prose line; assert truncation on a long manifest
        // description instead to exercise the cap.
        let cargo = format!("[package]\nname=\"a\"\nversion=\"0.1.0\"\ndescription=\"{long}\"\n");
        let fs2 = FakeFs::default().file("/repo/Cargo.toml", &cargo);
        let facts2 = gather(repo(), &fs2, &FakeGit::default());
        assert_eq!(facts.description.as_deref(), Some("intro"));
        // Count characters, not bytes — the cap is a char cap.
        assert_eq!(
            facts2.description.as_deref().map(|d| d.chars().count()),
            Some(120)
        );
    }

    #[test]
    fn read_limit_counts_chars_not_bytes() {
        // A multibyte description right at the boundary: a byte-slice cap would
        // truncate/corrupt it; the char cap keeps it whole. `く` is 3 bytes.
        let desc = "く".repeat(60); // 60 chars, 180 bytes — under the 120 char cap
        let cargo = format!("[package]\nname=\"a\"\nversion=\"0.1.0\"\ndescription=\"{desc}\"\n");
        let fs = FakeFs::default().file("/repo/Cargo.toml", &cargo);
        let facts = gather(repo(), &fs, &FakeGit::default());
        assert_eq!(facts.description.as_deref(), Some(desc.as_str()));
        // No replacement character crept in from a mid-codepoint byte slice.
        assert!(!facts.description.as_deref().unwrap().contains('\u{FFFD}'));
    }

    // ── SemVer tag handling ────────────────────────────────────────────────

    #[test]
    fn semver_parse_plain_prefixed_and_prerelease() {
        assert_eq!(semver_parse("v1.2.3"), Some((1, 2, 3, false)));
        assert_eq!(semver_parse("1.2.3"), Some((1, 2, 3, false)));
        assert_eq!(semver_parse("core-v1.2.3"), Some((1, 2, 3, false)));
        assert_eq!(semver_parse("@acme/cli@2.0.0"), Some((2, 0, 0, false)));
        assert_eq!(semver_parse("1.2.3-rc1"), Some((1, 2, 3, true)));
        assert_eq!(semver_parse("1.2.3+build"), Some((1, 2, 3, false)));
        assert_eq!(semver_parse("nightly"), None);
        assert_eq!(semver_parse("1.2"), None);
        assert_eq!(semver_parse("1.2.3.4"), None);
    }

    #[test]
    fn ge_1_0_release_from_tag_but_not_from_prerelease() {
        let fs = FakeFs::default().file(
            "/repo/Cargo.toml",
            "[package]\nname=\"a\"\nversion=\"0.9.0\"\n",
        );
        // A 1.0.0 tag → has_ge_1_0_release.
        let facts = gather(repo(), &fs, &git_with(1, 1, &["v0.9.0", "v1.0.0"]));
        assert!(facts.has_semver_tag);
        assert!(facts.has_ge_1_0_release);
        // Only a 1.0.0-rc prerelease tag → not a >=1.0 release.
        let fs2 = FakeFs::default().file(
            "/repo/Cargo.toml",
            "[package]\nname=\"a\"\nversion=\"0.9.0\"\n",
        );
        let facts2 = gather(repo(), &fs2, &git_with(1, 1, &["v1.0.0-rc1"]));
        assert!(facts2.has_semver_tag);
        assert!(!facts2.has_ge_1_0_release);
    }

    #[test]
    fn ge_1_0_release_from_manifest_version() {
        let fs = FakeFs::default().file(
            "/repo/Cargo.toml",
            "[package]\nname=\"a\"\nversion=\"1.4.0\"\n",
        );
        let facts = gather(repo(), &fs, &FakeGit::default());
        assert!(facts.has_ge_1_0_release);
    }

    #[test]
    fn version_ge_1_0_requires_dot_after_major() {
        assert!(version_ge_1_0(Some("1.0.0")));
        assert!(version_ge_1_0(Some("v2.3")));
        assert!(version_ge_1_0(Some("2024.1")));
        assert!(!version_ge_1_0(Some("0.9.9")));
        assert!(!version_ge_1_0(Some("1"))); // no dot
        assert!(!version_ge_1_0(None));
    }

    // ── Maturity truth table ───────────────────────────────────────────────

    #[test]
    fn production_needs_two_recent_committers_ge_1_0_and_ci() {
        let fs = FakeFs::default()
            .file(
                "/repo/Cargo.toml",
                "[package]\nname=\"a\"\nversion=\"1.2.0\"\n",
            )
            .file("/repo/.github/workflows/ci.yml", "on: push\n");
        let facts = gather(repo(), &fs, &git_with(4, 3, &["v1.2.0"]));
        assert!(facts.has_ci);
        assert!(facts.has_ge_1_0_release);
        assert!(facts.maturity_signals.production);
        assert_eq!(facts.inferred_maturity, Maturity::Production);
    }

    #[test]
    fn mvp_when_ci_present_but_not_production_grade() {
        // Has CI (so not spike) but only one recent committer and no >=1.0.
        let fs = FakeFs::default()
            .file(
                "/repo/Cargo.toml",
                "[package]\nname=\"a\"\nversion=\"0.3.0\"\n",
            )
            .file("/repo/.github/workflows/ci.yml", "on: push\n");
        let facts = gather(repo(), &fs, &git_with(1, 1, &["v0.3.0"]));
        assert!(!facts.maturity_signals.production);
        assert!(!facts.maturity_signals.spike);
        assert_eq!(facts.inferred_maturity, Maturity::Mvp);
    }

    #[test]
    fn spike_forced_by_readme_label_even_with_multiple_committers() {
        // No CI, no SemVer tag, but 3 committers — the README label flips spike.
        let fs = FakeFs::default()
            .file(
                "/repo/Cargo.toml",
                "[package]\nname=\"a\"\nversion=\"0.1.0\"\n",
            )
            .file(
                "/repo/README.md",
                "# X\n\nThis is an experimental prototype.\n",
            );
        let facts = gather(repo(), &fs, &git_with(3, 3, &[]));
        assert_eq!(facts.readme_self_label.as_deref(), Some("spike"));
        assert!(facts.maturity_signals.spike);
        assert_eq!(facts.inferred_maturity, Maturity::Spike);
    }

    #[test]
    fn multi_committer_no_ci_no_label_is_mvp_not_spike() {
        // No CI, no tag, >1 committer, no label → spike's committer clause fails
        // → mvp (the tie-breaker).
        let fs = FakeFs::default().file(
            "/repo/Cargo.toml",
            "[package]\nname=\"a\"\nversion=\"0.1.0\"\n",
        );
        let facts = gather(repo(), &fs, &git_with(3, 2, &[]));
        assert!(!facts.maturity_signals.spike);
        assert_eq!(facts.inferred_maturity, Maturity::Mvp);
    }

    // ── Determinism ────────────────────────────────────────────────────────

    #[test]
    fn same_repo_same_facts() {
        let build = || {
            FakeFs::default()
                .file(
                    "/repo/Cargo.toml",
                    "[package]\nname=\"a\"\nversion=\"0.3.0\"\n",
                )
                .file("/repo/.github/workflows/ci.yml", "on: push\n")
        };
        let a = gather(repo(), &build(), &git_with(2, 2, &["v0.3.0"]));
        let b = gather(repo(), &build(), &git_with(2, 2, &["v0.3.0"]));
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }
}
