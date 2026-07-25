//! Build script: capture the current git commit into `OSSCTL_GIT_COMMIT` so
//! `ossctl version` can report it (`AGENTS-AI-FIRST-CLI.md` §10). Falls back to
//! `"unknown"` outside a git checkout (e.g. a source tarball build).

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .find(|p| p.join(".git").exists())
        .map(PathBuf::from);
    let repo = repo_root.as_deref().unwrap_or(&manifest_dir);

    let commit = git(repo, &["rev-parse", "HEAD"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=OSSCTL_GIT_COMMIT={commit}");

    // Refresh the commit env var when HEAD moves. We let git resolve the exact
    // metadata paths (`rev-parse --git-path`) rather than reimplementing the
    // repository layout: in a linked worktree the per-worktree HEAD lives in the
    // worktree gitdir, but branch refs live in the *common* dir, and a packed
    // ref (`git gc`) has no loose file at all. `--git-path` handles all three;
    // hand-joining paths against the worktree gitdir (the naive approach) would
    // watch a path that never changes and serve a stale commit forever.
    if let Some(head_path) = git_path(repo, "HEAD") {
        println!("cargo:rerun-if-changed={head_path}");
    }
    // Packed refs have no loose file; watch the pack so a commit after `git gc`
    // still invalidates. Watching a not-yet-existent path is a harmless no-op.
    if let Some(packed) = git_path(repo, "packed-refs") {
        println!("cargo:rerun-if-changed={packed}");
    }
    // The loose file for the currently checked-out branch (resolved to the
    // common dir for worktrees). Detached HEAD has no symbolic ref → skip.
    if let Some(symref) = git(repo, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(ref_path) = git_path(repo, &symref) {
            println!("cargo:rerun-if-changed={ref_path}");
        }
    }
}

/// Run `git -C <repo> <args>` and return trimmed stdout on success.
fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8(out.stdout).ok())
        .flatten()
        .map(|s| s.trim().to_string())
}

/// Resolve `name` (e.g. `HEAD`, `packed-refs`, `refs/heads/main`) to an
/// absolute filesystem path via git, correct for both plain and worktree
/// checkouts. `None` if git is unavailable or the lookup fails.
fn git_path(repo: &Path, name: &str) -> Option<String> {
    git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-path", name],
    )
    .filter(|s| !s.is_empty())
}
