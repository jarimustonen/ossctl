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

    /// Block for `dur` before returning — the passage of real time, injected so
    /// waits are deterministic and instant under test.
    ///
    /// The release engine's crates.io index-wait (the multi-crate workspace
    /// publish path, [`crate::release::adapters::cargo`]) polls between attempts
    /// through this method. The default performs a genuine
    /// [`std::thread::sleep`], so the production [`Clock`] waits for real without
    /// implementing anything extra; a deterministic test fake overrides it to
    /// advance a virtual clock instead of sleeping, so a bounded-wait loop
    /// terminates instantly and without a real delay.
    fn sleep(&self, dur: std::time::Duration) {
        std::thread::sleep(dur);
    }
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
    /// Whether `path` exists and is a *regular file* — not a directory, FIFO,
    /// socket, or device (mirrors `os.path.isfile`). The facts detector gates
    /// every manifest/config read on this so a non-regular node named
    /// `Cargo.toml` neither marks an ecosystem nor blocks [`Self::read`] (a
    /// blocking `read` on a FIFO is a real hang the `exists && !is_dir`
    /// approximation would not prevent).
    fn is_file(&self, path: &std::path::Path) -> bool;
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
    /// The **common** git directory (`git rev-parse --git-common-dir`), resolved
    /// to an absolute path. The release journal roots its state under here so all
    /// linked worktrees of one repo share a single release-state root, and
    /// submodules / bare repos / `GIT_DIR` overrides resolve correctly — a
    /// literal `.git/` concatenation is *not* a portable substitute (ADR-0003
    /// §3, the panel's repeated correctness landmine). `Err` on any git failure.
    fn git_common_dir(&self) -> io::Result<std::path::PathBuf>;
}

/// Durable, atomic, lockable storage for the release journal — the seam that
/// keeps the event-sourced journal (ADR-0003) testable without touching the real
/// filesystem, while pinning down the atomicity discipline its production impl
/// **must** honor.
///
/// The append-then-apply contract (ADR-0003 §2, borrowed from `octl-core`) maps
/// onto these operations:
///
/// 1. [`Self::append_line`] fsyncs the event so it is durable **before** the
///    reducer applies it — a crash between append and apply replays as a clean
///    no-op-or-apply, because the journal (read back by [`Self::read_lines`]) is
///    the single source of truth.
/// 2. [`Self::write_atomic`] persists the derived manifest via temp-file → flush
///    → atomic rename → directory fsync, so a torn write can never leave a
///    half-written manifest (it is disposable and rebuildable regardless).
/// 3. [`Self::lock_exclusive`] enforces a single active cut per repo (a `flock`
///    on the releases-dir `.lock`): a concurrent cut/resume fails fast rather
///    than corrupting a run.
///
/// The port is deliberately path-driven (the journal computes paths via
/// [`crate::release::journal::JournalPaths`]); the impl adds no policy, only the
/// durability guarantees documented per method.
pub trait JournalStore {
    /// Take the single-active-cut exclusive lock at `lock_path` (creating parent
    /// directories as needed). The returned guard holds the lock until dropped.
    /// `Err` with [`io::ErrorKind::WouldBlock`] when another holder is active, so
    /// the caller can fail fast and name the active run.
    fn lock_exclusive(&self, lock_path: &std::path::Path) -> io::Result<Box<dyn JournalLock>>;
    /// Append `line` (one serialized event, no embedded newline — the store adds
    /// the single trailing `\n`) to the JSONL file at `path`, creating the file
    /// and parent directories if absent, and **fsync** so it is durable before
    /// returning. This is the append half of append-then-apply.
    ///
    /// The write must be **atomic at line granularity**: on return the line is
    /// either fully present or not present, never truncated. The production impl
    /// opens with `O_APPEND`, `write_all`s the line + newline, and fsyncs the file
    /// **and** — when it created the file or a parent directory — the containing
    /// directory, so a newly created `RunCreated` survives power loss (fsyncing
    /// only the file leaves the new directory entry non-durable). A torn *final*
    /// line from a hard kill mid-write is still possible in theory; recovering it
    /// (truncate-to-last-good under the lock) is a documented follow-up, not part
    /// of this port yet — [`crate::release::journal::read_events`] currently
    /// rejects any malformed line.
    fn append_line(&self, path: &std::path::Path, line: &str) -> io::Result<()>;
    /// Read every line of the JSONL file at `path`. `Ok(vec![])` when the file is
    /// absent (a not-yet-written journal is empty, not an error).
    fn read_lines(&self, path: &std::path::Path) -> io::Result<Vec<String>>;
    /// Read the full contents of the (atomically-written) file at `path`, or
    /// `Ok(None)` when it is absent. Used for the torn-free fast-path read of the
    /// `manifest.json` cache; unlike [`Self::read_lines`] it returns raw bytes.
    fn read(&self, path: &std::path::Path) -> io::Result<Option<Vec<u8>>>;
    /// Atomically replace the file at `path` with `bytes`: write a temp file in
    /// the same directory, flush + fsync it, rename it over `path`, then fsync the
    /// directory. Creates parent directories as needed.
    fn write_atomic(&self, path: &std::path::Path, bytes: &[u8]) -> io::Result<()>;
    /// The immediate entry *names* (not full paths) within `dir`, or `Ok(vec![])`
    /// when `dir` is absent — used to enumerate run-id subdirectories for
    /// `release list`.
    fn list_dir(&self, dir: &std::path::Path) -> io::Result<Vec<String>>;
}

/// An opaque RAII guard for the single-active-cut lock taken by
/// [`JournalStore::lock_exclusive`]. Dropping it releases the lock; there are no
/// methods — its lifetime *is* the contract.
pub trait JournalLock {}

/// Creates and publishes the **one** shared release tag + GitHub Release for a
/// cut — the external side of the coordinator's coordinator-only tag phase
/// (ADR-0002 §2).
///
/// Tagging is deliberately **not** on the [`crate::release::adapters::ReleaseAdapter`]
/// trait: no per-ecosystem adapter can create the shared tag, which is what makes
/// "tag once, after every publish succeeds" a structural guarantee. The
/// coordinator drives these three steps in order and journals each as its own
/// resumable event (`tag_created_local` → `tag_pushed_remote` →
/// `github_release_created`), so an interrupted tag phase resumes from the first
/// incomplete step rather than re-tagging.
///
/// Every method is **idempotent-friendly**: the coordinator only calls a step
/// whose journalled fact is not yet present, but a production impl should still
/// treat "already exists" as success (a pushed tag that is already on the remote,
/// a Release that already exists) rather than an error, so a resume after a crash
/// *between* the external action and its journal write reconciles cleanly.
pub trait Tagger {
    /// Create the annotated tag `tag` (message `message`) pointing at the sealed
    /// `commit` in the local repository.
    ///
    /// `commit` is the plan's sealed `HEAD` (not whatever `HEAD` happens to be at
    /// tag time), so the tag authenticates the approved commit even if `HEAD`
    /// moved during the cut. Idempotent-friendly: if the tag already exists **at
    /// `commit`** this is success (a resumed cut re-reaching this step after a
    /// crash between the tag and its journal write); a tag that exists pointing
    /// **elsewhere** is a genuine conflict (`Err`), never silently overwritten.
    fn create_tag(&self, tag: &str, commit: &str, message: &str) -> io::Result<()>;
    /// Push the already-created tag `tag` to the remote. Idempotent-friendly: an
    /// already-present identical remote tag is success; `Err` on a real push
    /// failure (network, auth, a conflicting remote ref).
    fn push_tag(&self, tag: &str) -> io::Result<()>;
    /// Create the GitHub Release for `tag` (titled `title`), returning its URL
    /// when the host reports one. Idempotent-friendly: an already-existing Release
    /// is success (returning its URL); `Err` on a real creation failure.
    fn create_github_release(&self, tag: &str, title: &str) -> io::Result<Option<String>>;
}
