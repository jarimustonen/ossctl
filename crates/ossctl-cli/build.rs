//! Build script: capture the current git commit into `OSSCTL_GIT_COMMIT` so
//! `ossctl version` can report it (`AGENTS-AI-FIRST-CLI.md` §10). Falls back to
//! `"unknown"` outside a git checkout (e.g. a source tarball build).

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .find(|p| p.join(".git").exists())
        .map(PathBuf::from);

    let commit = Command::new("git")
        .arg("-C")
        .arg(repo_root.as_deref().unwrap_or(&manifest_dir))
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            o.status
                .success()
                .then(|| String::from_utf8(o.stdout).ok())
                .flatten()
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=OSSCTL_GIT_COMMIT={commit}");

    // Refresh the commit env var when HEAD moves. In a worktree, `.git` is a
    // *file* (`gitdir: <path>` pointing at `.git/worktrees/<name>/`); both the
    // dir and file layouts are handled.
    if let Some(root) = repo_root {
        let git_path = root.join(".git");
        let git_dir = if git_path.is_dir() {
            Some(git_path)
        } else if git_path.is_file() {
            std::fs::read_to_string(&git_path)
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("gitdir:").map(str::trim).map(str::to_string))
                })
                .map(PathBuf::from)
                .map(|p| if p.is_absolute() { p } else { root.join(p) })
        } else {
            None
        };

        if let Some(git_dir) = git_dir {
            let head_path = git_dir.join("HEAD");
            if let Ok(head) = std::fs::read_to_string(&head_path) {
                println!("cargo:rerun-if-changed={}", head_path.display());
                if let Some(rest) = head.strip_prefix("ref: ") {
                    let ref_path = git_dir.join(rest.trim());
                    println!("cargo:rerun-if-changed={}", ref_path.display());
                }
            }
        }
    }
}
