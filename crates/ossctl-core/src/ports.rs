//! Injected effect ports — the seam that keeps every domain testable in
//! isolation without touching the real filesystem, git, network, or clock
//! (ADR-0001 §2).
//!
//! Domain code (`contract`, `facts`, `audit`, `release`) takes these traits by
//! reference instead of calling `std::process`, `std::time`, or the network
//! directly. Production supplies real implementations in `ossctl-cli`; tests
//! supply deterministic fakes. At founding these are the **trait shapes** only;
//! concrete implementations land with their consuming units.

use std::io;

/// Output of a subprocess run through a [`CommandRunner`].
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Process exit status code (`None` if terminated by a signal).
    pub status: Option<i32>,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

/// Runs external commands (git, package-manager, registry CLIs) on behalf of a
/// domain, capturing their output. The single seam for shelling out — nothing
/// in `ossctl-core` calls `std::process::Command` directly.
pub trait CommandRunner {
    /// Run `program` with `args` in `cwd`, capturing stdout/stderr.
    fn run(&self, program: &str, args: &[&str], cwd: &std::path::Path)
        -> io::Result<CommandOutput>;
}

/// Supplies the current time. Injected so time-dependent logic (journal
/// timestamps, run ages) is deterministic under test.
pub trait Clock {
    /// Current time as whole seconds since the Unix epoch.
    fn now_unix(&self) -> u64;
}

/// Generates opaque, unique, non-deterministic identifiers — run ids and the
/// like. Injected so id-dependent output is deterministic under test.
///
/// Note: this is **not** the source of `plan_id`. A release plan id is
/// *content-addressed* — derived deterministically from the sealed plan's
/// canonical bytes (ADR-0002), not generated here. Do not route plan sealing
/// through this port.
pub trait IdGen {
    /// Produce a fresh, unique identifier.
    fn new_id(&self) -> String;
}

/// Read/write access to the filesystem — the `Fs` half of the `Fs/Git` seam
/// (ADR-0001 §2). Domain code (contract loading, journal persistence, sealed
/// plans) goes through this port rather than calling `std::fs` directly, so it
/// is testable against an in-memory fake. At founding this is a deliberately
/// small surface; it grows (atomic writes, dir listing, metadata) as the
/// journal and plan-sealing units land.
pub trait Fs {
    /// Read a file's full contents as bytes.
    fn read(&self, path: &std::path::Path) -> io::Result<Vec<u8>>;
    /// Whether `path` exists.
    fn exists(&self, path: &std::path::Path) -> bool;
    /// Whether `path` exists and is a directory (used for the contract's
    /// fragment-dir producer check, which is a *directory*, not a file).
    fn is_dir(&self, path: &std::path::Path) -> bool;
    /// List the immediate entry *names* (not full paths) within `dir` — the
    /// facts detector's CI probe needs to know whether `.github/workflows`
    /// holds at least one entry, not merely that the directory exists.
    /// `Ok(vec![])` for an empty directory; `Err` when `dir` is absent or
    /// unreadable (the detector treats that as "no entries").
    fn read_dir(&self, dir: &std::path::Path) -> io::Result<Vec<String>>;
}

/// Queries a package registry for already-published state — the "remote is
/// ground truth" source the release reconciler consults (ADR-0003).
pub trait RegistryQuery {
    /// Versions of `package` already published to `ecosystem`'s registry.
    fn published_versions(&self, ecosystem: &str, package: &str) -> io::Result<Vec<String>>;
}

/// Read-only view of the git repository under audit/release. The detector and
/// release engine read repo facts through this port rather than shelling out
/// to `git` themselves.
///
/// The detector's queries are deliberately *best-effort*: an unborn or
/// non-repository root makes several of these fail, and the detector treats
/// every such failure as "absent" (no committers, no tags) rather than an
/// error — mirroring the read-only, never-mutating `infer-repo-facts.py`.
pub trait GitRepo {
    /// The full commit hash of `HEAD`. `Err` on an unborn repository (no
    /// commits yet) or a non-repository root — the detector reads this as
    /// "the repo has no commits".
    fn head_commit(&self) -> io::Result<String>;
    /// Whether the root is inside a git work tree (`git rev-parse
    /// --is-inside-work-tree` succeeds). `false` for a non-repository root or
    /// on any git error.
    fn is_work_tree(&self) -> bool;
    /// Raw `git shortlog -sne --all HEAD` output — one line per distinct
    /// committer. When `since` is set (a git date expression such as
    /// `"1 year ago"`), the log is limited to commits at or after it. `Err` on
    /// any git failure (an unborn/empty repo); the detector counts that as zero
    /// committers.
    fn shortlog(&self, since: Option<&str>) -> io::Result<String>;
    /// The repository's tag names (`git tag --list`), trimmed and with empty
    /// lines dropped. `Err` on any git failure; the detector reads that as "no
    /// tags".
    fn tags(&self) -> io::Result<Vec<String>>;
}
