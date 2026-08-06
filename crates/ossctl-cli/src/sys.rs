//! Concrete, production implementations of the `ossctl-core` effect ports.
//!
//! `ossctl-core` domains take the [`ossctl_core::ports`] traits by reference so
//! they are testable against in-memory fakes; this module supplies the real
//! ones backed by `std`. [`RealFs`] backs the [`Fs`] port (the contract reader
//! and the facts detector); [`RealGitRepo`] backs the read-only [`GitRepo`]
//! port for the facts detector by shelling out to `git`. The remaining
//! registry/clock ports gain real impls with their consuming units.

use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ossctl_core::ports::{
    Clock, CommandOutput, CommandRunner, Fs, GitRepo, IdGen, JournalLock, JournalStore,
    RegistryQuery, Tagger,
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

/// The real wall clock, backing the [`Clock`] port with system time. Whole
/// seconds since the Unix epoch; a clock set before the epoch degrades to `0`
/// rather than panicking.
pub struct RealClock;

impl Clock for RealClock {
    fn now_unix(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// The real run-id generator, backing the [`IdGen`] port with a ULID-shaped
/// identifier (ADR-0003 §3): a 48-bit millisecond timestamp followed by 80 bits
/// of entropy, rendered as 26 Crockford base-32 characters (lexicographically
/// sortable by creation time).
///
/// The entropy is derived from the system clock's sub-second component, a
/// process-lifetime counter, and a stack address hashed together — **not** a CSPRNG.
/// This is deliberate: `ossctl` takes no `rand`/`ulid` dependency (the workspace
/// `Cargo.toml` is a hot file), and a release run id needs only to be unique on
/// one machine, where the single-active-cut lock already serializes concurrent
/// cuts. Collisions would require two runs in the same millisecond with a hash
/// collision — not a correctness hazard for a per-repo, human-paced operation.
pub struct RealIdGen;

/// Monotonic within one process, to diversify same-millisecond ids.
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

impl IdGen for RealIdGen {
    fn new_id(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let ms = u64::try_from(now.as_millis()).unwrap_or(u64::MAX) & ((1 << 48) - 1);
        let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        // A stack address gives cheap per-call address-space entropy without a
        // dependency; hashed, never dereferenced.
        let anchor = 0u8;
        let addr = std::ptr::addr_of!(anchor) as usize;

        // The pid distinguishes two short-lived processes that start in the same
        // millisecond with a similar stack layout (the case a time+addr hash alone
        // could collide on).
        let pid = std::process::id();
        let mut h1 = DefaultHasher::new();
        (now.subsec_nanos(), counter, ms, addr, pid).hash(&mut h1);
        let mut h2 = DefaultHasher::new();
        (counter, h1.finish(), addr, ms, pid).hash(&mut h2);
        // 80 bits of entropy: 64 from h1, 16 more from h2.
        let rand80 = (u128::from(h1.finish()) << 16) | u128::from(h2.finish() & 0xffff);

        let value = (u128::from(ms) << 80) | (rand80 & ((1 << 80) - 1));
        crockford_u128(value)
    }
}

/// Encode a 128-bit value as 26 Crockford base-32 characters (the ULID text
/// form). The top two of the 130 encodable bits are always zero here.
fn crockford_u128(mut value: u128) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut buf = [0u8; 26];
    for slot in buf.iter_mut().rev() {
        *slot = ALPHABET[(value & 0x1f) as usize];
        value >>= 5;
    }
    // `buf` is ASCII by construction.
    String::from_utf8(buf.to_vec()).expect("crockford alphabet is ASCII")
}

/// The real registry-state lookup, backing the read-only [`RegistryQuery`] port
/// the release reconciler consults (the remote is ground truth, ADR-0003).
///
/// The reconciler degrades a lookup failure to [`VerifyOutcome::Unknown`], never a
/// false `Missing`, so an ecosystem with no wired query is honestly "cannot
/// check" rather than "did not land". Two ecosystems are wired: `node` (queried
/// through the clean `npm view … versions --json` surface) and `rust` (queried
/// against the crates.io **sparse index** at `index.crates.io`). The remaining
/// ecosystems return an `Err` so the reconcile reports `unknown` until their
/// registry query lands — matching the skeleton state of the adapter layer.
///
/// **Timeout gap (partially closed).** Like [`RealCommandRunner`] and
/// [`RealGitRepo`], the `npm` arm has no hard wall-clock timeout — `std` has none
/// on `Command::output` and this crate takes no new dependency, so a stalled
/// `npm`/DNS/TLS can hang it. The `rust` arm shells to `curl` with an explicit
/// `--max-time`, so it is bounded; a bounded-deadline `npm` query is a documented
/// follow-up.
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

    /// The crates.io **sparse index** entry for `crate_name`, relative to the
    /// index root (`index.crates.io`) — the cargo index-directory convention
    /// (`config.json` "dl"/"api" aside): 1-char names live under `1/`, 2-char
    /// under `2/`, 3-char under `3/<first-char>/`, and everything else under
    /// `<first-two>/<next-two>/`. The name is lowercased; hyphens and underscores
    /// are preserved (the index keeps them distinct). Crate names are ASCII by
    /// [`Self::validate_crate_name`], so the byte slicing here never splits a
    /// multi-byte char.
    fn sparse_index_path(crate_name: &str) -> String {
        let name = crate_name.to_ascii_lowercase();
        match name.len() {
            0 => name,
            1 => format!("1/{name}"),
            2 => format!("2/{name}"),
            3 => format!("3/{}/{}", &name[0..1], name),
            _ => format!("{}/{}/{}", &name[0..2], &name[2..4], name),
        }
    }

    /// The crates.io ceiling on a crate name (its `max-name-length`); a name
    /// longer than this cannot exist on the registry, so a longer input is a
    /// tampered/erroneous journal value, not a lookup — refuse it before it
    /// becomes an oversized URL.
    const MAX_CRATE_NAME_LEN: usize = 64;

    /// Reject a crate name that could distort the query — a leading `-` (which a
    /// CLI would read as a flag) or any character outside the crates.io-permitted
    /// set (`[A-Za-z0-9_-]`). Mirrors the flag-injection guard on the `npm` arm,
    /// but stricter: the name is interpolated into a URL, so anything that could
    /// alter the request path (`/`, `?`, `.`, whitespace, …) is refused rather
    /// than percent-mangled into a wrong-but-successful lookup. Also caps the
    /// length at [`Self::MAX_CRATE_NAME_LEN`] so a tampered journal cannot drive
    /// an unbounded request URL.
    fn validate_crate_name(crate_name: &str) -> io::Result<()> {
        if crate_name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to query crates.io for an empty crate name",
            ));
        }
        if crate_name.len() > Self::MAX_CRATE_NAME_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "refusing to query crates.io for an over-long crate name ({} > {} bytes)",
                    crate_name.len(),
                    Self::MAX_CRATE_NAME_LEN
                ),
            ));
        }
        if crate_name.starts_with('-')
            || !crate_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("refusing to query crates.io for a suspicious crate name: {crate_name:?}"),
            ));
        }
        Ok(())
    }

    /// Parse a crates.io sparse-index body (JSON-lines, one release per line) into
    /// the list of published version strings, cross-checking every record's crate
    /// `name` against `expected_name`.
    ///
    /// **Yanked versions are included.** A yanked version still occupies its
    /// version slot on crates.io — re-publishing that exact `crate@version` is
    /// *rejected* — so for the defer/idempotency predicate a yanked version is
    /// unambiguously "already published" and must count.
    ///
    /// **Fails closed on anything anomalous.** A line that is not valid JSON, one
    /// missing the string `name`/`vers` fields, or one whose `name` is not
    /// `expected_name` (a cache-poisoned or misrouted body for the *wrong* crate)
    /// is an `Err`, never a silent drop that could misread as "not published". An
    /// empty result is *not* decided here — [`Self::parse_sparse_response`] rejects
    /// a `200` that parsed to zero versions, since a live crate's index is never
    /// empty.
    fn parse_sparse_index(body: &str, expected_name: &str) -> io::Result<Vec<String>> {
        /// The subset of a sparse-index release record this query reads. Serde
        /// streams past the unread fields (`deps`, `cksum`, `features`, …) without
        /// allocating them; a record missing `name` or `vers` fails to deserialize
        /// and so fails closed.
        #[derive(serde::Deserialize)]
        struct SparseEntry {
            name: String,
            vers: String,
        }

        let mut versions = Vec::new();
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let entry: SparseEntry = serde_json::from_str(line).map_err(|e| {
                io::Error::other(format!(
                    "crates.io sparse-index line was not valid JSON: {e}"
                ))
            })?;
            if !entry.name.eq_ignore_ascii_case(expected_name) {
                return Err(io::Error::other(format!(
                    "crates.io sparse-index body was for crate {:?}, not the requested {expected_name:?}",
                    entry.name
                )));
            }
            versions.push(entry.vers);
        }
        Ok(versions)
    }

    /// Classify a completed `curl` transaction to the sparse index by its HTTP
    /// status (appended after the body via `--write-out`, behind
    /// [`Self::HTTP_CODE_MARKER`]) and, for a `200`, parse the body for
    /// `expected_name`.
    ///
    /// - `200` ⇒ parse the body into versions; a `200` that yields *zero* versions
    ///   is anomalous (a live crate's sparse index always carries at least one
    ///   release — even a fully-yanked crate keeps its lines), so it fails
    ///   **closed** with `Err` rather than the "not published" `Ok(vec![])` a
    ///   truncated transfer or a proxy-intercepted empty body would otherwise mint.
    /// - `404` ⇒ the crate has no published versions yet — the one authoritative
    ///   "missing" signal, `Ok(vec![])`, **not** an error.
    /// - `410` (Gone), `000` (curl never completed a transfer), or any other
    ///   status ⇒ the registry state is unknown, so fail **closed** with `Err` —
    ///   never an empty `Vec`, which the reconciler would read as "not published"
    ///   and could act on with an unsafe publish. (`410` in particular does *not*
    ///   prove a crate was never published.)
    fn parse_sparse_response(stdout: &str, expected_name: &str) -> io::Result<Vec<String>> {
        let (body, code) = stdout.rsplit_once(Self::HTTP_CODE_MARKER).ok_or_else(|| {
            io::Error::other("curl did not report an HTTP status for the crates.io sparse index")
        })?;
        match code.trim() {
            "200" => {
                let versions = Self::parse_sparse_index(body, expected_name)?;
                if versions.is_empty() {
                    return Err(io::Error::other(
                        "crates.io sparse index returned HTTP 200 with no release records; \
                         treating as unknown rather than 'not published'",
                    ));
                }
                Ok(versions)
            }
            "404" => Ok(Vec::new()),
            other => Err(io::Error::other(format!(
                "crates.io sparse index returned an unexpected HTTP status {other}"
            ))),
        }
    }

    /// Sentinel that separates the sparse-index response body from the trailing
    /// HTTP status `curl --write-out` appends, so both arrive on one stdout
    /// capture yet stay unambiguously splittable ([`Self::parse_sparse_response`]).
    const HTTP_CODE_MARKER: &'static str = "\n__OSSCTL_HTTP_CODE__:";

    /// Query the crates.io sparse index for the published versions of
    /// `crate_name`.
    ///
    /// Shells to `curl` (no new HTTP dependency, mirroring the `npm` arm's
    /// shell-out style) with a bounded `--max-time`, stdin `/dev/null`, and the
    /// HTTP status appended after the body. A network-level failure (DNS,
    /// connection refused, timeout — `curl` exits non-zero, HTTP status `000`)
    /// fails **closed** with `Err`; a `404` is the genuine "not yet published"
    /// signal (`Ok(vec![])`). `curl` completing an HTTP transaction (any status)
    /// exits `0`, so status classification — not the process exit — drives the
    /// decision (see [`Self::parse_sparse_response`]).
    ///
    /// Hardening on the invocation: `--disable` ignores an ambient `~/.curlrc`
    /// that could otherwise alter the output shape and break the status-marker
    /// split; redirects are deliberately **not** followed (no `--location`) — the
    /// sparse index is a flat file host that answers `200`/`404` directly, so a
    /// `3xx` is anomalous and must fail closed rather than chase a cross-host
    /// redirect to a `404` that would misread as "not published". `curl` being
    /// absent surfaces as a named `Err` (still fail-closed), not a silent miss.
    fn crates_io_versions(crate_name: &str) -> io::Result<Vec<String>> {
        Self::validate_crate_name(crate_name)?;
        let url = format!(
            "https://index.crates.io/{}",
            Self::sparse_index_path(crate_name)
        );
        let write_out = format!("{}%{{http_code}}", Self::HTTP_CODE_MARKER);
        let out = Command::new("curl")
            .args([
                "--disable",
                "--silent",
                "--show-error",
                "--max-time",
                "30",
                "--user-agent",
                concat!("ossctl/", env!("CARGO_PKG_VERSION")),
            ])
            .arg("--write-out")
            .arg(&write_out)
            .arg(&url)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| {
                io::Error::other(format!(
                    "could not run `curl` to query the crates.io sparse index \
                     (is curl installed?): {e}"
                ))
            })?;
        if !out.status.success() {
            // Network-level failure (DNS/connect/timeout): curl exits non-zero and
            // reports HTTP status 000. Fail CLOSED — never read as "not published".
            return Err(io::Error::other(format!(
                "could not reach the crates.io sparse index for {crate_name:?}: curl exited {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        Self::parse_sparse_response(&stdout, crate_name)
    }
}

impl RegistryQuery for RealRegistryQuery {
    fn published_versions(&self, ecosystem: &str, package: &str) -> io::Result<Vec<String>> {
        match ecosystem {
            "node" => Self::npm_versions(package),
            "rust" => Self::crates_io_versions(package),
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

/// The real tag publisher, backing the coordinator-only [`Tagger`] port by
/// shelling out to `git` (local tag + push) and `gh` (GitHub Release), rooted at
/// the repository. Hardened against non-interactive hangs the same way
/// [`RealGitRepo`] and [`RealCommandRunner`] are (no terminal/askpass/`gh`
/// prompts). No wall-clock timeout — the same accepted `std`-has-none gap.
pub struct RealTagger {
    root: PathBuf,
}

impl RealTagger {
    /// A tagger operating on the repository at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Run `program` (git/gh) with `args` in the repo root, prompts disabled.
    fn run(&self, program: &str, args: &[&str]) -> io::Result<std::process::Output> {
        Command::new(program)
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "true")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .output()
    }

    /// Map a non-zero exit into an `Err` carrying the captured diagnostic (stderr,
    /// or stdout when the tool wrote its error there).
    fn check(out: std::process::Output, what: &str) -> io::Result<std::process::Output> {
        if out.status.success() {
            return Ok(out);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = if stderr.trim().is_empty() {
            String::from_utf8_lossy(&out.stdout).into_owned()
        } else {
            stderr.into_owned()
        };
        Err(io::Error::other(format!(
            "{what} exited {:?}: {}",
            out.status.code(),
            detail.trim()
        )))
    }

    /// The commit a local tag resolves to (`git rev-parse <tag>^{commit}`), or
    /// `None` if the tag does not exist. Used to make [`Self::create_tag`]
    /// idempotent: an already-present tag at the sealed commit is success, one
    /// pointing elsewhere is a conflict.
    fn tag_commit(&self, tag: &str) -> Option<String> {
        let out = self
            .run(
                "git",
                &["rev-parse", "--verify", "-q", &format!("{tag}^{{commit}}")],
            )
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!sha.is_empty()).then_some(sha)
    }
}

impl Tagger for RealTagger {
    fn create_tag(&self, tag: &str, commit: &str, message: &str) -> io::Result<()> {
        // Idempotent resume: a tag already at the sealed commit is success; one at
        // a different commit is a genuine conflict we must never overwrite.
        if let Some(existing) = self.tag_commit(tag) {
            if existing == commit || existing.starts_with(commit) || commit.starts_with(&existing) {
                return Ok(());
            }
            return Err(io::Error::other(format!(
                "tag `{tag}` already exists at {existing}, not the sealed commit {commit}"
            )));
        }
        // Tag the sealed commit explicitly (`git tag -a <tag> <commit>`), not HEAD.
        let out = self.run("git", &["tag", "-a", tag, commit, "-m", message])?;
        Self::check(out, "git tag")?;
        Ok(())
    }

    fn push_tag(&self, tag: &str) -> io::Result<()> {
        // A fully-qualified refspec is unambiguous; an already-present identical
        // remote tag reports "Everything up-to-date" (exit 0), so re-pushing on
        // resume is a no-op rather than an error.
        let refspec = format!("refs/tags/{tag}:refs/tags/{tag}");
        let out = self.run("git", &["push", "origin", &refspec])?;
        Self::check(out, "git push")?;
        Ok(())
    }

    fn create_github_release(&self, tag: &str, title: &str) -> io::Result<Option<String>> {
        // Idempotent resume: if the Release already exists, return its URL rather
        // than failing on a duplicate `gh release create`.
        let view = self.run(
            "gh",
            &["release", "view", tag, "--json", "url", "-q", ".url"],
        )?;
        if view.status.success() {
            let url = String::from_utf8_lossy(&view.stdout).trim().to_string();
            return Ok((!url.is_empty()).then_some(url));
        }
        let out = self.run(
            "gh",
            &[
                "release",
                "create",
                tag,
                "--title",
                title,
                "--generate-notes",
            ],
        )?;
        let out = Self::check(out, "gh release create")?;
        // `gh release create` prints the Release URL on stdout.
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok((!url.is_empty()).then_some(url))
    }
}

/// The real durable release-journal store, backing the [`JournalStore`] port with
/// `std::fs`. Honors the append-then-apply atomicity discipline (ADR-0003 §2):
/// events are `O_APPEND`-written and fsynced before returning; the manifest is
/// replaced via temp-file → fsync → atomic rename → directory fsync.
///
/// **Lock deviation (accepted, documented follow-up).** ADR-0003 §3 specifies a
/// `flock`; this impl instead uses an `O_EXCL` lock *file* so `ossctl` takes no
/// new dependency (`std::fs::File::lock` is newer than the pinned MSRV, and a
/// `libc`/`fs2` dep would edit the hot workspace `Cargo.toml`). The trade-off: a
/// hard process kill leaves a stale lock file that a human (or a future `doctor
/// --fix`) must remove, whereas `flock` releases on death. Mutual exclusion under
/// normal operation (and Drop-based release) is equivalent.
pub struct RealJournalStore;

/// The `O_EXCL` lock guard: removes its lock file on drop (normal release).
struct RealJournalLock {
    path: PathBuf,
}

impl JournalLock for RealJournalLock {}

impl Drop for RealJournalLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Best-effort directory fsync so a newly created file's directory entry is
/// durable (ADR-0003 §2). Silently ignored where a directory cannot be opened as
/// a file (non-Unix); the data fsync already happened.
fn fsync_dir(dir: &Path) {
    let _ = File::open(dir).and_then(|f| f.sync_all());
}

impl JournalStore for RealJournalStore {
    fn lock_exclusive(&self, lock_path: &Path) -> io::Result<Box<dyn JournalLock>> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(mut f) => {
                // Record the holder's pid for diagnostics / stale-lock recovery.
                let _ = writeln!(f, "{}", std::process::id());
                let _ = f.sync_all();
                Ok(Box::new(RealJournalLock {
                    path: lock_path.to_path_buf(),
                }))
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "another release cut/resume holds the single-active-cut lock",
            )),
            Err(e) => Err(e),
        }
    }

    fn append_line(&self, path: &Path, line: &str) -> io::Result<()> {
        let created_parent = match path.parent() {
            Some(p) if !p.exists() => {
                std::fs::create_dir_all(p)?;
                true
            }
            _ => false,
        };
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        if created_parent {
            if let Some(p) = path.parent() {
                fsync_dir(p);
            }
        }
        Ok(())
    }

    fn read_lines(&self, path: &Path) -> io::Result<Vec<String>> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(s.lines().map(str::to_string).collect()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "manifest path has no parent")
        })?;
        std::fs::create_dir_all(parent)?;
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("manifest.json");
        let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(".{file_name}.{}.{counter}.tmp", std::process::id()));
        {
            let mut f = File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        fsync_dir(parent);
        Ok(())
    }

    fn list_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                let mut names = Vec::new();
                for entry in entries {
                    names.push(entry?.file_name().to_string_lossy().into_owned());
                }
                names.sort();
                Ok(names)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The sparse-index directory-prefix convention, exactly as cargo lays it out.
    #[test]
    fn sparse_index_path_follows_cargo_prefix_convention() {
        assert_eq!(RealRegistryQuery::sparse_index_path("a"), "1/a");
        assert_eq!(RealRegistryQuery::sparse_index_path("no"), "2/no");
        assert_eq!(RealRegistryQuery::sparse_index_path("cfg"), "3/c/cfg");
        assert_eq!(RealRegistryQuery::sparse_index_path("serde"), "se/rd/serde");
        // ossctl's own crates — the dogfood targets.
        assert_eq!(
            RealRegistryQuery::sparse_index_path("ossctl-core"),
            "os/sc/ossctl-core"
        );
        // The name is lowercased; hyphens are preserved.
        assert_eq!(
            RealRegistryQuery::sparse_index_path("Ossctl-Core"),
            "os/sc/ossctl-core"
        );
    }

    #[test]
    fn parse_sparse_index_collects_versions_including_yanked() {
        // A trimmed-down but shape-accurate sparse-index body: three releases, the
        // middle one yanked. All three must appear — a yanked version still holds
        // its slot, so re-publishing it would be rejected.
        let body = concat!(
            r#"{"name":"tool","vers":"0.9.0","yanked":false}"#,
            "\n",
            r#"{"name":"tool","vers":"1.0.0","yanked":true}"#,
            "\n",
            r#"{"name":"tool","vers":"1.1.0","yanked":false}"#,
            "\n",
        );
        let versions = RealRegistryQuery::parse_sparse_index(body, "tool").unwrap();
        assert_eq!(versions, vec!["0.9.0", "1.0.0", "1.1.0"]);
    }

    #[test]
    fn parse_sparse_index_blank_lines_are_skipped() {
        let body = "\n{\"name\":\"tool\",\"vers\":\"1.0.0\",\"yanked\":false}\n\n";
        assert_eq!(
            RealRegistryQuery::parse_sparse_index(body, "tool").unwrap(),
            vec!["1.0.0"]
        );
    }

    #[test]
    fn parse_sparse_index_rejects_malformed_line() {
        // A line with no `vers` field must fail closed, not silently drop.
        let body = r#"{"name":"tool","yanked":false}"#;
        assert!(RealRegistryQuery::parse_sparse_index(body, "tool").is_err());
        // A line with no `name` field is equally corrupt and fails closed.
        let body = r#"{"vers":"1.0.0","yanked":false}"#;
        assert!(RealRegistryQuery::parse_sparse_index(body, "tool").is_err());
        // Non-JSON garbage likewise fails closed.
        assert!(RealRegistryQuery::parse_sparse_index("not json", "tool").is_err());
    }

    #[test]
    fn parse_sparse_index_rejects_a_wrong_crate_body() {
        // A body whose records are for a *different* crate (a cache-poisoned or
        // misrouted 200) must fail closed, never yield versions for the wrong name.
        let body = r#"{"name":"other","vers":"1.0.0","yanked":false}"#;
        assert!(RealRegistryQuery::parse_sparse_index(body, "tool").is_err());
        // The requested-name comparison is case-insensitive (crates.io answers the
        // canonical case; the request may carry another) — this must NOT be an error.
        let body = r#"{"name":"Tool","vers":"1.0.0","yanked":false}"#;
        assert_eq!(
            RealRegistryQuery::parse_sparse_index(body, "tool").unwrap(),
            vec!["1.0.0"]
        );
    }

    #[test]
    fn parse_sparse_response_200_returns_versions() {
        let stdout = format!(
            "{}{}200",
            r#"{"name":"tool","vers":"1.0.0","yanked":false}"#,
            RealRegistryQuery::HTTP_CODE_MARKER
        );
        assert_eq!(
            RealRegistryQuery::parse_sparse_response(&stdout, "tool").unwrap(),
            vec!["1.0.0"]
        );
    }

    #[test]
    fn parse_sparse_response_404_is_empty_not_error() {
        // 404 = crate has never been published = the legitimate "missing" signal.
        let stdout = format!("{}404", RealRegistryQuery::HTTP_CODE_MARKER);
        assert_eq!(
            RealRegistryQuery::parse_sparse_response(&stdout, "tool").unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn parse_sparse_response_empty_200_fails_closed() {
        // A 200 with no release records is anomalous (a live crate's index is never
        // empty) — a truncated transfer or a proxy-intercepted empty body. It must
        // fail closed, NOT return the "not published" Ok(vec![]).
        let stdout = format!("{}200", RealRegistryQuery::HTTP_CODE_MARKER);
        assert!(RealRegistryQuery::parse_sparse_response(&stdout, "tool").is_err());
        // Whitespace-only body is likewise empty-after-parse and fails closed.
        let stdout = format!("   \n  {}200", RealRegistryQuery::HTTP_CODE_MARKER);
        assert!(RealRegistryQuery::parse_sparse_response(&stdout, "tool").is_err());
    }

    #[test]
    fn parse_sparse_response_unreachable_fails_closed() {
        // HTTP status 000: curl never completed a transfer (DNS/connect failure).
        // Must be Err, never an empty Vec that would read as "not published".
        let stdout = format!("{}000", RealRegistryQuery::HTTP_CODE_MARKER);
        assert!(RealRegistryQuery::parse_sparse_response(&stdout, "tool").is_err());
        // A server-side 5xx is equally "unknown" and fails closed.
        let stdout = format!("{}503", RealRegistryQuery::HTTP_CODE_MARKER);
        assert!(RealRegistryQuery::parse_sparse_response(&stdout, "tool").is_err());
        // 410 Gone does NOT prove a crate was never published — it is unknown, not
        // "missing", so it fails closed rather than returning Ok(vec![]).
        let stdout = format!("{}410", RealRegistryQuery::HTTP_CODE_MARKER);
        assert!(RealRegistryQuery::parse_sparse_response(&stdout, "tool").is_err());
        // An unfollowed redirect (no --location) is anomalous for a flat-file index
        // and must not be chased; classified as unexpected → fail closed.
        let stdout = format!("{}301", RealRegistryQuery::HTTP_CODE_MARKER);
        assert!(RealRegistryQuery::parse_sparse_response(&stdout, "tool").is_err());
        // A response with no status marker at all (curl produced no write-out)
        // cannot be classified and must fail closed rather than guess.
        assert!(RealRegistryQuery::parse_sparse_response("some body, no marker", "tool").is_err());
    }

    #[test]
    fn validate_crate_name_rejects_suspicious_input() {
        assert!(RealRegistryQuery::validate_crate_name("ossctl-core").is_ok());
        assert!(RealRegistryQuery::validate_crate_name("serde_json").is_ok());
        // A name at the length ceiling is fine; one byte over is refused.
        assert!(RealRegistryQuery::validate_crate_name(
            &"a".repeat(RealRegistryQuery::MAX_CRATE_NAME_LEN)
        )
        .is_ok());
        assert!(RealRegistryQuery::validate_crate_name(
            &"a".repeat(RealRegistryQuery::MAX_CRATE_NAME_LEN + 1)
        )
        .is_err());
        // Empty, flag-like, path-traversing, or otherwise out-of-charset names.
        assert!(RealRegistryQuery::validate_crate_name("").is_err());
        assert!(RealRegistryQuery::validate_crate_name("-oops").is_err());
        assert!(RealRegistryQuery::validate_crate_name("a/b").is_err());
        assert!(RealRegistryQuery::validate_crate_name("a b").is_err());
        assert!(RealRegistryQuery::validate_crate_name("a.b").is_err());
    }
}
