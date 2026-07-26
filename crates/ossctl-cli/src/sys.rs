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
use std::time::{SystemTime, UNIX_EPOCH};

use ossctl_core::ports::{
    Clock, CommandOutput, CommandRunner, Fs, GitRepo, JournalLock, JournalStore, RegistryQuery,
};

/// The real subprocess runner, backing the [`CommandRunner`] port with
/// `std::process`. The audit's read-only GitHub community-standards lookup
/// (`git remote get-url origin`, then `gh api …/community/profile`) runs through
/// this. Hardened against non-interactive hangs the same way [`RealGitRepo`] is:
/// stdin is `/dev/null` and terminal/askpass/`gh` prompts are disabled, so a
/// command that would block on a credential or auth prompt fails fast instead.
/// `GH_NO_UPDATE_NOTIFIER` keeps `gh`'s update banner out of the captured
/// stderr the audit surfaces as a diagnostic. A command that cannot spawn (`gh`
/// not installed) surfaces as an `Err`, which the audit reads as "could not
/// check" ⇒ `unknown`, never `false`.
///
/// **Timeout gap (accepted).** Like [`RealGitRepo::git`], there is no hard
/// wall-clock timeout — `std` has none on `Command::output` and this crate takes
/// no new dependency. `gh api` is a network call, so a stalled DNS/TLS/proxy can
/// hang the audit; the prompt-disabling above removes the common interactive
/// stall, and the read-only queries are cheap on a healthy connection.
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run(&self, program: &str, args: &[&str], cwd: &Path) -> io::Result<CommandOutput> {
        let out = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "true")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .output()?;
        Ok(CommandOutput {
            status: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

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

    fn git_common_dir(&self) -> io::Result<PathBuf> {
        let raw = self.git_stdout(&["rev-parse", "--git-common-dir"])?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(io::Error::other(
                "git rev-parse --git-common-dir produced no output",
            ));
        }
        // git may return a path relative to the repo root (e.g. `.git`); resolve
        // it against `root` so callers always get an absolute location.
        let path = Path::new(trimmed);
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        })
    }
}

/// The real wall clock, backing the [`Clock`] port with `SystemTime`.
pub struct RealClock;

impl Clock for RealClock {
    fn now_unix(&self) -> u64 {
        // A pre-epoch system clock is not a real deployment; clamp to 0 rather
        // than panic so a misconfigured host cannot crash a read-only command.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// The real registry-state lookup, backing the read-only [`RegistryQuery`] port
/// the release reconciler consults (the remote is ground truth, ADR-0003).
///
/// The reconciler degrades a lookup failure to [`VerifyOutcome::Unknown`], never a
/// false `Missing`, so an ecosystem with no wired query is honestly "cannot
/// check" rather than "did not land". Today only `node` (queried through the
/// clean `npm view … versions --json` surface) is wired; the remaining ecosystems
/// return an `Err` so the reconcile reports `unknown` until their registry query
/// lands — matching the skeleton state of the adapter layer.
///
/// **Timeout gap (accepted).** Like [`RealCommandRunner`] and [`RealGitRepo`],
/// there is no hard wall-clock timeout — `std` has none on `Command::output` and
/// this crate takes no new dependency. A stalled `npm`/DNS/TLS can hang the
/// lookup; a bounded-deadline registry client is a documented follow-up.
///
/// [`VerifyOutcome::Unknown`]: ossctl_core::protocol::release::VerifyOutcome::Unknown
pub struct RealRegistryQuery;

impl RealRegistryQuery {
    /// `npm view --json <package> versions` → the published version list.
    ///
    /// Hardened against non-interactive hangs the same way the other real ports
    /// are (stdin `/dev/null`, update notifier disabled). A missing `npm`, a
    /// non-zero exit, or unparsable output all surface as `Err`, which the
    /// reconciler reads as `unknown` (never a false `missing`).
    fn npm_versions(package: &str) -> io::Result<Vec<String>> {
        // A package name is a positional; one that begins with '-' would be read
        // by npm as a flag (flag injection from a tampered/erroneous journal).
        // A real npm package name never starts with '-', so reject it outright
        // rather than let it alter where/how the query runs.
        if package.starts_with('-') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "refusing to query npm for a package name that looks like a flag: {package:?}"
                ),
            ));
        }
        let out = Command::new("npm")
            .args(["view", "--json", package, "versions"])
            .stdin(Stdio::null())
            .env("NO_UPDATE_NOTIFIER", "1")
            .env("NPM_CONFIG_FUND", "false")
            .output()?;
        if !out.status.success() {
            return Err(io::Error::other(format!(
                "npm view {package} versions exited {:?}",
                out.status.code()
            )));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        // `npm view` returns a JSON array for many versions, or a bare JSON string
        // for a single one; accept either, but reject any other shape (an object,
        // null, or a mixed array) rather than silently dropping entries — a
        // partial parse that yielded an empty list would misread as `missing`.
        match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
            Ok(serde_json::Value::Array(items)) => items
                .into_iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| io::Error::other("npm returned a non-string version entry"))
                })
                .collect(),
            Ok(serde_json::Value::String(v)) => Ok(vec![v]),
            _ => Err(io::Error::other(format!(
                "could not parse `npm view --json {package} versions` output"
            ))),
        }
    }
}

impl RegistryQuery for RealRegistryQuery {
    fn published_versions(&self, ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        match ecosystem {
            "node" => Self::npm_versions(package),
            other => Err(io::Error::other(format!(
                "no registry query wired for ecosystem '{other}' yet"
            ))),
        }
    }
}

/// A **read-only** [`JournalStore`] for the `release verify`/`show` path.
///
/// `verify` reconciles a journaled run without ever writing — no manifest
/// self-heal, no lock, no publish — so its store deliberately implements only the
/// read operations. The mutating operations ([`JournalStore::lock_exclusive`],
/// [`JournalStore::append_line`], [`JournalStore::write_atomic`]) return an error
/// rather than a fake success: the writable, lockable production store belongs to
/// the `release cut`/`resume` units, and routing a mutation through the read-only
/// store is a programming error worth surfacing loudly.
pub struct ReadOnlyJournalStore;

impl ReadOnlyJournalStore {
    fn read_only(op: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{op} is not available on the read-only journal store"),
        )
    }
}

impl JournalStore for ReadOnlyJournalStore {
    fn lock_exclusive(&self, _lock_path: &Path) -> io::Result<Box<dyn JournalLock>> {
        Err(Self::read_only("lock_exclusive"))
    }

    fn append_line(&self, _path: &Path, _line: &str) -> io::Result<()> {
        Err(Self::read_only("append_line"))
    }

    fn read_lines(&self, path: &Path) -> io::Result<Vec<String>> {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                // `verify` reads without a lock, so a `release cut` may be
                // mid-append. Every *committed* line ends in '\n' (the store
                // appends it), so a trailing fragment with no newline is an
                // in-progress, not-yet-durable append — return only the bytes up
                // to the last '\n' so a torn tail is a dropped partial rather than
                // a false `journal_unreadable` corruption error.
                let end = contents.rfind('\n').map_or(0, |i| i + 1);
                Ok(contents[..end].lines().map(str::to_string).collect())
            }
            // An absent journal is empty, not an error (mirrors the port contract).
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write_atomic(&self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
        Err(Self::read_only("write_atomic"))
    }

    fn list_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries {
                    names.push(entry?.file_name().to_string_lossy().into_owned());
                }
            }
            // An absent releases root simply has no runs.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        }
        names.sort();
        Ok(names)
    }
}
