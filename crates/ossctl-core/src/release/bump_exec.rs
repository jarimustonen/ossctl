//! Cut-time execution of the sealed version-bump phase (`release-rust-workspace-
//! multicrate` facet 2/3) — the effectful half whose pure text transforms live in
//! [`crate::release::bump`].
//!
//! [`apply_bump`] runs **inside the coordinator's clean checkout of the sealed commit**
//! (`release-cut-clean-checkout`), before the build barrier, so every later phase
//! builds and publishes the bumped tree. It:
//!
//! 1. sets `[workspace.package] version`, or a plain root `[package] version` when the
//!    workspace-inheritance source is absent;
//! 2. rewrites each sealed intra-workspace `=`-pin set (verifying every declaration is
//!    equivalent, then updating all of them; fail closed on zero/non-equivalence,
//!    [`crate::release::bump::rewrite_pin`]);
//! 3. refreshes `Cargo.lock` (`cargo update --workspace`);
//! 4. finalizes the CHANGELOG (`[Unreleased]` → a dated section) when the contract's
//!    changelog mode asked for it;
//! 5. runs any contract-declared `bump_hook` (see the execution contract below);
//! 6. commits the edits and returns the **bump commit sha** — the commit the tag points
//!    at (not the pre-bump sealed HEAD).
//!
//! Every step **fails closed**: a failed edit, lockfile refresh, hook, post-hook
//! validation, or commit aborts the cut with a [`BumpExecError`] before the build
//! barrier — nothing external has happened yet (no publish, no tag).
//!
//! # File I/O
//!
//! Like the [`homebrew`](crate::release::adapters::homebrew) adapter, this reads and
//! writes manifests/CHANGELOG with `std::fs` directly (a manifest edit has no CLI to
//! route through [`CommandRunner`](crate::ports::CommandRunner)); all writes are confined to the throwaway sealed
//! checkout the coordinator just materialized. Process effects (`cargo`, the hook,
//! `git`) go through the injected [`CommandRunner`](crate::ports::CommandRunner), so the executor is unit-testable
//! against a real temp checkout + a recording fake runner.
//!
//! # `bump_hook` execution contract (supply-chain surface, schema.rs)
//!
//! A declared `bump_hook` is **arbitrary code the engine runs during the release**,
//! equivalent in trust to a `build.rs` the cut already compiles. Its purpose is to
//! regenerate version-embedding artifacts (test snapshots that embed the version) so
//! they do not go stale on the bump and red CI. The contract this executor honors:
//!
//! - **Invocation:** `sh -c "<hook>"` with the hook string passed as a **single,
//!   verbatim** argv element — **no** dynamic data (version, package names) is ever
//!   interpolated into it, so there is no shell-injection surface from cut-time data
//!   (schema.rs:399). The hook is surfaced verbatim as a plan-time reviewer warning, so
//!   an approver has seen exactly what runs.
//! - **Working directory:** the sealed checkout root.
//! - **Environment:** the cut's ambient environment (inherited), the same trust
//!   boundary as a `build.rs`. Secrets the cut carries (registry tokens) are reachable —
//!   this is why the hook is an eyes-on supply-chain surface, not a sandbox.
//! - **Permitted effect:** regenerating derived files. **Post-hook validation** re-reads
//!   the workspace manifest and rejects the cut if the hook reverted or altered the
//!   version bump ([`BumpExecError::HookViolatedVersion`]) — the one invariant that must
//!   survive, since a hook that changed the version would publish the wrong number. A
//!   full permitted-path allowlist is intentionally not enforced (snapshot regen writes
//!   an open-ended set of test files); the version invariant is the load-bearing check.
//! - **Failure:** a non-zero exit fails the cut closed ([`BumpExecError::Hook`]).
//! - **Timeout:** the [`CommandRunner`](crate::ports::CommandRunner) port carries no timeout today, so the hook runs
//!   to completion (a CI-level job timeout is the backstop). A first-class per-command
//!   timeout is a tracked follow-up.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::protocol::plan::BumpPlan;
use crate::release::adapters::EffectCtx;
use crate::release::bump::{self, BumpEditError};

/// The outcome of an applied bump: the commit the edits landed in (the tag target) and
/// the CHANGELOG effective date (journalled for resume reuse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BumpOutcome {
    /// The bump commit sha — the commit the release tag points at.
    pub commit: String,
    /// The `YYYY-MM-DD` date the CHANGELOG was finalized under.
    pub effective_date: String,
}

/// Why the cut-time bump could not be applied. Every variant is a fail-closed refusal
/// **before** the build barrier — no publish or tag has happened.
#[derive(Debug)]
pub enum BumpExecError {
    /// A pure edit transform refused (missing root manifest version, a non-matching or
    /// ambiguous pin, or an absent `## [Unreleased]` section).
    Edit(BumpEditError),
    /// A manifest/CHANGELOG file could not be read or written in the checkout.
    Fs {
        /// The offending path.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A pin rewrite named a workspace member whose manifest could not be located in
    /// the checkout (a `package` the workspace graph resolved but whose directory the
    /// executor could not map). Fail closed rather than skip a sealed pin.
    MemberManifestNotFound {
        /// The dependent crate whose manifest was expected.
        package: String,
    },
    /// `cargo update --workspace` (the lockfile refresh) failed.
    LockRefresh(String),
    /// The declared `bump_hook` exited non-zero.
    Hook {
        /// The exit code (or a signal note).
        status: String,
        /// Captured stderr, trimmed.
        stderr: String,
    },
    /// The `bump_hook` altered the root release version away from the bumped value — a
    /// contract violation (the hook may regenerate derived artifacts, not re-version).
    HookViolatedVersion {
        /// The version the bump set.
        expected: String,
        /// What the manifest read after the hook.
        found: String,
    },
    /// A `git` step (add / commit / rev-parse / push) failed.
    Git(String),
}

impl std::fmt::Display for BumpExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Edit(e) => write!(f, "{e}"),
            Self::Fs { path, source } => {
                write!(
                    f,
                    "cannot access `{}` in the sealed checkout: {source}",
                    path.display()
                )
            }
            Self::MemberManifestNotFound { package } => write!(
                f,
                "the bump names crate `{package}` but its manifest could not be located in the \
                 sealed checkout"
            ),
            Self::LockRefresh(m) => write!(f, "refreshing Cargo.lock failed: {m}"),
            Self::Hook { status, stderr } => {
                write!(f, "the bump_hook failed ({status}): {stderr}")
            }
            Self::HookViolatedVersion { expected, found } => write!(
                f,
                "the bump_hook changed the root manifest version to `{found}`, but the bump set \
                 `{expected}` — refusing to publish a hook-altered version"
            ),
            Self::Git(m) => write!(f, "git step failed during the bump: {m}"),
        }
    }
}

impl std::error::Error for BumpExecError {}

impl From<BumpEditError> for BumpExecError {
    fn from(e: BumpEditError) -> Self {
        Self::Edit(e)
    }
}

/// Apply the sealed `bump` inside `ctx.repo_root` (the coordinator's clean checkout),
/// committing the edits and returning the [`BumpOutcome`]. `effective_date` is the
/// `YYYY-MM-DD` CHANGELOG date (freshly computed on a first run, or the journalled date
/// on resume so it never re-dates).
///
/// The bump commit is created on the checkout's detached HEAD; its object lives in the
/// shared store, so the coordinator's tagger (running against the real repo) can point
/// the release tag at it within the same cut. Advancing the real repo's *branch* to the
/// bump commit is deliberately **not** done here (a best-effort pre-publish push would be
/// an externally-visible effect before the build barrier, and a failed one leaves a
/// split-brain remote) — it is a documented follow-up (`release-rust-workspace-multicrate`).
///
/// # Errors
/// [`BumpExecError`] on any failed edit, lockfile refresh, hook, post-hook validation,
/// or git step — always **before** the build barrier, so nothing external happened.
pub fn apply_bump(
    ctx: &EffectCtx<'_>,
    bump: &BumpPlan,
    effective_date: &str,
) -> Result<BumpOutcome, BumpExecError> {
    let root = ctx.repo_root;

    // 1. Set the root release version (verified against `from_version`). A workspace
    //    package version is authoritative when present; only an absent workspace source
    //    falls back to a plain single-crate `[package]` version.
    let root_manifest = root.join("Cargo.toml");
    let text = read(&root_manifest)?;
    let bumped = if bump::workspace_version(&text).is_some() {
        bump::set_workspace_version(&text, &bump.from_version, &bump.to_version)?
    } else {
        bump::set_package_version(&text, &bump.from_version, &bump.to_version)?
    };
    write(&root_manifest, &bumped)?;

    // 2. Rewrite each sealed intra-workspace pin set in its dependent crate's manifest,
    //    verifying all declarations are the exact old value (fail closed on zero or a
    //    non-equivalent declaration).
    if !bump.pin_rewrites.is_empty() {
        let members = member_manifest_paths(root)?;
        for pin in &bump.pin_rewrites {
            let manifest = members.get(&pin.in_package).ok_or_else(|| {
                BumpExecError::MemberManifestNotFound {
                    package: pin.in_package.clone(),
                }
            })?;
            let text = read(manifest)?;
            let rewritten = bump::rewrite_pin(&text, &pin.dependency, &pin.from, &pin.to)?;
            write(manifest, &rewritten)?;
        }
    }

    // 3. Refresh Cargo.lock so its workspace-member entries carry the new version — but
    //    only when the repo tracks a lockfile (a library workspace that intentionally
    //    git-ignores Cargo.lock must not have one introduced by the cut). NOTE: this runs
    //    `cargo update --workspace`, which re-resolves within the manifests' semver ranges;
    //    for a version-only bump the third-party graph is unchanged in practice, but
    //    pinning the resolution deterministically is a documented follow-up.
    if root.join("Cargo.lock").exists() {
        refresh_lockfile(ctx)?;
    }

    // 4. Finalize the CHANGELOG when the contract's mode asked for it and a CHANGELOG
    //    exists. A declared-but-missing `## [Unreleased]` fails closed (via the transform).
    if bump.changelog_finalize {
        let changelog = root.join("CHANGELOG.md");
        if changelog.exists() {
            let text = read(&changelog)?;
            let finalized = bump::finalize_changelog(&text, &bump.to_version, effective_date)?;
            write(&changelog, &finalized)?;
        }
    }

    // 5. Run the declared bump_hook (supply-chain surface — see the module docs), then
    //    validate the hook did not alter the version.
    if let Some(hook) = &bump.bump_hook {
        run_hook(ctx, hook)?;
        let after = read(&root_manifest)?;
        let found = bump::root_manifest_version(&after);
        if found.as_deref() != Some(bump.to_version.as_str()) {
            return Err(BumpExecError::HookViolatedVersion {
                expected: bump.to_version.clone(),
                found: found.unwrap_or_default(),
            });
        }
    }

    // 6. Commit the edits in the checkout and read back the bump commit sha.
    let commit = commit_bump(ctx, &bump.to_version)?;

    Ok(BumpOutcome {
        commit,
        effective_date: effective_date.to_string(),
    })
}

/// Read a checkout file, mapping I/O errors to [`BumpExecError::Fs`].
fn read(path: &Path) -> Result<String, BumpExecError> {
    std::fs::read_to_string(path).map_err(|source| BumpExecError::Fs {
        path: path.to_path_buf(),
        source,
    })
}

/// Write a checkout file, mapping I/O errors to [`BumpExecError::Fs`].
fn write(path: &Path, contents: &str) -> Result<(), BumpExecError> {
    std::fs::write(path, contents).map_err(|source| BumpExecError::Fs {
        path: path.to_path_buf(),
        source,
    })
}

/// Refresh `Cargo.lock`'s workspace-member entries to the bumped version via
/// `cargo update --workspace` (dependencies untouched).
fn refresh_lockfile(ctx: &EffectCtx<'_>) -> Result<(), BumpExecError> {
    let out = ctx
        .runner
        .run("cargo", &["update", "--workspace"], ctx.repo_root)
        .map_err(|e| BumpExecError::LockRefresh(format!("cannot run cargo: {e}")))?;
    if out.status != Some(0) {
        return Err(BumpExecError::LockRefresh(format!(
            "exit {}: {}",
            status_str(out.status),
            out.stderr.trim()
        )));
    }
    Ok(())
}

/// Run the contract-declared `bump_hook` as `sh -c "<hook>"` in the checkout — the
/// verbatim string as a single argv element, no interpolation (see the module docs).
fn run_hook(ctx: &EffectCtx<'_>, hook: &str) -> Result<(), BumpExecError> {
    let out = ctx
        .runner
        .run("sh", &["-c", hook], ctx.repo_root)
        .map_err(|e| BumpExecError::Hook {
            status: "spawn failed".to_string(),
            stderr: e.to_string(),
        })?;
    if out.status != Some(0) {
        return Err(BumpExecError::Hook {
            status: status_str(out.status),
            stderr: out.stderr.trim().to_string(),
        });
    }
    Ok(())
}

/// `git add -A` then `git commit` the bump edits in the checkout, returning the new
/// commit sha (`git rev-parse HEAD`).
fn commit_bump(ctx: &EffectCtx<'_>, version: &str) -> Result<String, BumpExecError> {
    let root = ctx.repo_root;
    run_git(ctx, &["add", "-A"], root)?;
    let message = format!("release: v{version}");
    run_git(ctx, &["commit", "-m", &message], root)?;
    let out = ctx
        .runner
        .run("git", &["rev-parse", "HEAD"], root)
        .map_err(|e| BumpExecError::Git(format!("rev-parse HEAD: {e}")))?;
    if out.status != Some(0) {
        return Err(BumpExecError::Git(format!(
            "rev-parse HEAD exit {}: {}",
            status_str(out.status),
            out.stderr.trim()
        )));
    }
    let sha = out.stdout.trim().to_string();
    if sha.is_empty() {
        return Err(BumpExecError::Git(
            "git rev-parse HEAD returned no commit sha after the bump commit".to_string(),
        ));
    }
    Ok(sha)
}

/// Run a `git` subcommand in `cwd`, failing closed on a non-zero exit.
fn run_git(ctx: &EffectCtx<'_>, args: &[&str], cwd: &Path) -> Result<(), BumpExecError> {
    let out = ctx
        .runner
        .run("git", args, cwd)
        .map_err(|e| BumpExecError::Git(format!("`git {}`: {e}", args.join(" "))))?;
    if out.status != Some(0) {
        return Err(BumpExecError::Git(format!(
            "`git {}` exit {}: {}",
            args.join(" "),
            status_str(out.status),
            out.stderr.trim()
        )));
    }
    Ok(())
}

/// Map member crate names to their manifest paths by scanning the workspace root's
/// `[workspace] members` (explicit paths + trailing single-level globs), reading each
/// `[package].name`. Mirrors the facts detector's member resolution enough for the
/// lib+bin shape; an unresolved dependent fails the pin rewrite closed.
fn member_manifest_paths(root: &Path) -> Result<BTreeMap<String, PathBuf>, BumpExecError> {
    let root_manifest = root.join("Cargo.toml");
    let text = read(&root_manifest)?;
    let mut map = BTreeMap::new();
    for rel in workspace_member_dirs(root, &text) {
        let manifest = root.join(&rel).join("Cargo.toml");
        let Ok(member_text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        if let Some(name) = package_name(&member_text) {
            map.insert(name, manifest);
        }
    }
    Ok(map)
}

/// The workspace member directories declared in a root manifest's `[workspace] members`
/// array — explicit entries plus a trailing single-level glob (`crates/*`) expanded by
/// listing that directory. A best-effort line scan matching the facts detector's shape.
fn workspace_member_dirs(root: &Path, root_text: &str) -> Vec<String> {
    let Some(members) = toml_string_array(root_text, "members") else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in members {
        if let Some(parent) = entry.strip_suffix("/*") {
            // Expand a single-level glob by listing the parent directory.
            if let Ok(read_dir) = std::fs::read_dir(root.join(parent)) {
                for e in read_dir.flatten() {
                    if e.path().is_dir() {
                        dirs.push(format!("{parent}/{}", e.file_name().to_string_lossy()));
                    }
                }
            }
        } else if !entry.contains('*') {
            dirs.push(entry);
        }
    }
    dirs
}

/// The `members = ["…", …]` string array under a `[workspace]` table, as owned strings.
/// A best-effort single-array scan (members are declared once, near the top).
fn toml_string_array(text: &str, key: &str) -> Option<Vec<String>> {
    // Find `<key> = [` and read until the closing `]` (possibly multi-line).
    let mut in_workspace = false;
    let mut collecting = false;
    let mut buf = String::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            in_workspace = h.trim() == "workspace";
            continue;
        }
        if collecting {
            buf.push_str(line);
            if line.contains(']') {
                break;
            }
            continue;
        }
        if in_workspace {
            if let Some(rest) = strip_key(t, key) {
                if let Some(after) = rest.trim_start().strip_prefix('[') {
                    buf.push_str(after);
                    if t.contains(']') {
                        break;
                    }
                    collecting = true;
                }
            }
        }
    }
    if buf.is_empty() && !collecting {
        return None;
    }
    let inner = buf.split(']').next().unwrap_or("");
    let items: Vec<String> = inner
        .split(',')
        .filter_map(|s| {
            let s = s.trim().trim_matches(['"', '\'']);
            (!s.is_empty()).then(|| s.to_string())
        })
        .collect();
    Some(items)
}

/// If `line` is `key = <rest>` (whole key), return `<rest>`; else `None`.
fn strip_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?;
    let rest = rest.trim_start();
    rest.strip_prefix('=')
}

/// The `[package].name` of a member manifest, or `None`.
fn package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            in_package = h.trim() == "package";
            continue;
        }
        if in_package {
            if let Some(rest) = strip_key(t, "name") {
                return Some(rest.trim().trim_matches(['"', '\'']).to_string());
            }
        }
    }
    None
}

/// A subprocess status rendered for an error message.
fn status_str(status: Option<i32>) -> String {
    status.map_or_else(|| "signal".to_string(), |c| c.to_string())
}

/// The UTC `YYYY-MM-DD` civil date for a Unix timestamp — the CHANGELOG effective date
/// the bump finalizes under. A self-contained `days → (y, m, d)` conversion (Howard
/// Hinnant's `civil_from_days`) so the release path needs no chrono dependency; the
/// injected [`Clock`](crate::ports::Clock) supplies the timestamp, so it is deterministic
/// under test.
#[must_use]
pub fn civil_date(unix_secs: u64) -> String {
    let days = i64::try_from(unix_secs / 86_400).unwrap_or(i64::MAX);
    // Shift the epoch to 0000-03-01 and compute the year-of-era / day-of-year.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests;
