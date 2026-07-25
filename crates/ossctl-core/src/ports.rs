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

/// Generates opaque unique identifiers (run ids, plan ids' random component).
/// Injected so id-dependent output is deterministic under test.
pub trait IdGen {
    /// Produce a fresh, unique identifier.
    fn new_id(&self) -> String;
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
pub trait GitRepo {
    /// The full commit hash of `HEAD`.
    fn head_commit(&self) -> io::Result<String>;
}
