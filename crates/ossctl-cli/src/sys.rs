//! Concrete, production implementations of the `ossctl-core` effect ports.
//!
//! `ossctl-core` domains take the [`ossctl_core::ports`] traits by reference so
//! they are testable against in-memory fakes; this module supplies the real
//! ones backed by `std`. [`RealFs`] backs the [`Fs`] port (the contract reader
//! and the facts detector); [`RealGitRepo`] backs the read-only [`GitRepo`]
//! port for the facts detector by shelling out to `git`. The remaining
//! registry/clock ports gain real impls with their consuming units.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ossctl_core::ports::{Fs, GitRepo};

/// The real filesystem, backing the [`Fs`] port with `std::fs`.
pub struct RealFs;

impl Fs for RealFs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn read_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        // Sort so the port yields a stable order (the in-memory fake sorts too);
        // callers must not depend on OS directory-iteration order.
        names.sort();
        Ok(names)
    }
}

/// The real git repository, backing the read-only [`GitRepo`] port by running
/// `git -C <root> …`. Every query is best-effort: a non-zero git exit (an
/// unborn or non-repository root) becomes an `Err`, which the detector reads as
/// "absent" — the port never mutates the repository.
pub struct RealGitRepo {
    root: PathBuf,
}

impl RealGitRepo {
    /// A git view rooted at `root` (the repository the facts are gathered from).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Run `git -C <root> <args>` and capture its output.
    ///
    /// Hardened against the realistic non-interactive hangs the Python detector
    /// dodges with its `timeout=15`: stdin is `/dev/null` and terminal/askpass
    /// prompts are disabled, so a git that would otherwise block on a credential
    /// or hook prompt fails fast instead. A hard wall-clock timeout (for a
    /// stalled network/NFS mount) is a remaining gap versus the Python — std has
    /// no timeout on `Command::output` and this crate takes no new dependency;
    /// the read-only queries here (`rev-parse`, `shortlog`, `tag`) do not touch
    /// the network on a healthy local repo.
    fn git(&self, args: &[&str]) -> io::Result<std::process::Output> {
        Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "true")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
    }

    /// The stdout of a git command, or an `Err` when it exits non-zero (so the
    /// detector's `.unwrap_or_default()` treats the signal as absent).
    fn git_stdout(&self, args: &[&str]) -> io::Result<String> {
        let out = self.git(args)?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(io::Error::other(format!(
                "git {} exited {:?}",
                args.join(" "),
                out.status.code()
            )))
        }
    }
}

impl GitRepo for RealGitRepo {
    fn head_commit(&self) -> io::Result<String> {
        let head = self.git_stdout(&["rev-parse", "HEAD"])?.trim().to_string();
        // An unborn repo can exit 0 with empty stdout; treat that as "no HEAD"
        // so the detector's `has_commits` gate reads false (matches the Python
        // `bool(... and _run_git(...))` truthiness check).
        if head.is_empty() {
            return Err(io::Error::other("git rev-parse HEAD produced no output"));
        }
        Ok(head)
    }

    fn is_work_tree(&self) -> bool {
        self.git(&["rev-parse", "--is-inside-work-tree"])
            .is_ok_and(|o| o.status.success())
    }

    fn shortlog(&self, since: Option<&str>) -> io::Result<String> {
        let mut args: Vec<String> = vec!["shortlog".into(), "-sne".into(), "--all".into()];
        if let Some(s) = since {
            args.push(format!("--since={s}"));
        }
        // An explicit revision keeps `git shortlog` from reading stdin (it would
        // otherwise block waiting for a piped log in a non-interactive run).
        args.push("HEAD".into());
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        self.git_stdout(&borrowed)
    }

    fn tags(&self) -> io::Result<Vec<String>> {
        Ok(self
            .git_stdout(&["tag", "--list"])?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }
}
