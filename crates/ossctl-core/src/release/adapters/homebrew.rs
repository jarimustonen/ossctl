//! Homebrew distribution adapter: `homebrew-tap` and `homebrew-core`.
//!
//! Updates a Homebrew formula (a custom tap, or a `homebrew-core` bump PR) so
//! `brew install` resolves the new version. A tap/core is not observable through
//! the [`RegistryQuery`](crate::ports::RegistryQuery) port, so `verify` returns
//! [`VerifyOutcome::Unknown`] **explicitly** rather than being excused from the
//! contract (ADR-0002 §1) — an honest "cannot check", never a false `Missing`.
//!
//! ## Create vs. bump (the first-formula bootstrap)
//!
//! `brew bump-formula-pr` *updates* a formula that already exists — on a fresh,
//! empty tap there is nothing to bump, so the very first release must **create**
//! the initial `<name>.rb` instead. This adapter chooses between the two paths by
//! asking the tap whether the formula already exists (through the injected
//! [`CommandRunner`](crate::ports::CommandRunner), so it is testable with no real
//! network or tap):
//!
//! - **absent** → the *create* path: generate a source-build formula (the release
//!   tarball's `url` + `sha256`, the license, a cargo build/install stanza), clone
//!   the tap, commit the new file on a branch, and open a PR.
//! - **present** → the *bump* path: `brew bump-formula-pr` carrying the release
//!   tarball's `--url` (+ `--sha256` when a digest is available).
//!
//! The create path only applies to a `homebrew-tap` target whose destination tap
//! the contract configured (`ctx.artifacts.homebrew.tap`). A `homebrew-core`
//! target — or a `homebrew-tap` with no resolved tap — falls back to the plain
//! bump path (first submission to `homebrew-core` is a human review process, not
//! an automated create).

use std::path::PathBuf;
use std::time::Duration;

use crate::contract::schema::Adapter;
use crate::protocol::release::{
    BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt, VerifyOutcome,
};

use super::{
    make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, HomebrewFormula, ReleaseAdapter,
    SourceTarball,
};

/// The homebrew distribution adapter, operating as `homebrew-tap` or
/// `homebrew-core`.
pub struct HomebrewAdapter {
    adapter: Adapter,
}

/// Which formula operation the adapter resolved for a target — the create path
/// (a first formula on an empty tap) or the bump path (an existing formula).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormulaPath {
    /// Generate + PR the initial `<name>.rb` (the tap has no formula yet).
    Create,
    /// `brew bump-formula-pr` an already-present formula.
    Bump,
}

impl HomebrewAdapter {
    /// Construct for a resolved homebrew adapter identity.
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        debug_assert!(matches!(
            adapter,
            Adapter::HomebrewTap | Adapter::HomebrewCore
        ));
        Self { adapter }
    }

    /// The destination tap slug (`owner/repo`) for a `homebrew-tap` create, when
    /// the contract configured one. `None` for `homebrew-core` or an unconfigured
    /// tap — which pins the adapter to the bump path (no bootstrap destination).
    fn tap<'a>(&self, artifacts: Option<&'a HomebrewFormula>) -> Option<&'a str> {
        if self.adapter != Adapter::HomebrewTap {
            return None;
        }
        artifacts.and_then(|h| h.tap.as_deref())
    }

    /// Decide the create-vs-bump path for `target`: a `homebrew-tap` with a
    /// configured tap probes the tap for the formula (create when absent); every
    /// other case is a bump.
    ///
    /// The probe runs through the injected runner (`gh api …/contents/…`); a
    /// non-zero exit (a `404`, typically) reads as *absent* → create. A spawn
    /// failure is a real error and is propagated.
    fn resolve_path(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<FormulaPath, AdapterError> {
        let homebrew = ctx.artifacts.homebrew.as_ref();
        match self.tap(homebrew) {
            Some(tap) if !Self::formula_exists(ctx, tap, &t.package)? => Ok(FormulaPath::Create),
            _ => Ok(FormulaPath::Bump),
        }
    }

    /// Ask the tap whether `Formula/<name>.rb` already exists, through the runner.
    ///
    /// Uses `gh api` against the tap's contents endpoint so no local checkout is
    /// needed for the probe. Only three outcomes are safe to act on:
    ///
    /// - exit `0` (the file is served) ⇒ **present** → bump.
    /// - a genuine `404` (`gh` renders it as `Not Found (HTTP 404)`) ⇒ **absent**
    ///   → create.
    /// - **anything else** — auth failure, rate-limit, network error, a private or
    ///   renamed tap, a 5xx — is an [`AdapterError::Command`], **not** "absent".
    ///   Treating an infrastructure error as absence would trigger a spurious
    ///   create that clones the tap and could overwrite an existing formula.
    ///
    /// A spawn failure (the port could not run `gh`) is a genuine
    /// [`AdapterError::Io`].
    fn formula_exists(ctx: &EffectCtx<'_>, tap: &str, name: &str) -> Result<bool, AdapterError> {
        let endpoint = format!("repos/{tap}/contents/Formula/{name}.rb");
        let cmd = PlannedCommand::new("gh", &["api", "--silent", &endpoint]);
        let out = ctx
            .runner
            .run("gh", &["api", "--silent", &endpoint], ctx.repo_root)
            .map_err(|e| AdapterError::Io {
                command: cmd.rendered(),
                source: e.to_string(),
            })?;
        if out.status == Some(0) {
            return Ok(true);
        }
        // A 404 is the only non-zero exit that means "absent". `gh` prints
        // `Not Found (HTTP 404)`; match the stable `404` token on either stream.
        if out.stderr.contains("404") || out.stdout.contains("404") {
            return Ok(false);
        }
        let detail = if out.stderr.trim().is_empty() {
            out.stdout
        } else {
            out.stderr
        };
        Err(AdapterError::Command {
            command: cmd.rendered(),
            code: out.status,
            stderr: detail,
        })
    }

    /// The `brew bump-formula-pr` command for an existing formula, carrying the
    /// threaded release tarball `--url` (+ `--sha256` when a digest is present).
    /// Options precede the `--` terminator so a formula name is never parsed as a
    /// flag. Unchanged from the pre-bootstrap behaviour.
    fn bump_command(&self, tarball: Option<&SourceTarball>, name: &str) -> PlannedCommand {
        let mut args: Vec<String> = match self.adapter {
            Adapter::HomebrewCore => vec!["bump-formula-pr".into(), "--no-fork".into()],
            _ => vec!["bump-formula-pr".into()],
        };
        if let Some(tarball) = tarball {
            args.push("--url".into());
            args.push(tarball.url.clone());
            if let Some(sha256) = &tarball.sha256 {
                args.push("--sha256".into());
                args.push(sha256.clone());
            }
        }
        args.push("--".into());
        args.push(name.to_string());
        PlannedCommand {
            program: "brew".into(),
            args,
        }
    }

    /// A **fresh, unpredictable** scratch checkout the create path clones the tap
    /// into. Unique per attempt (pid + a monotonic-ish nanosecond stamp) so:
    /// concurrent cuts/tests never collide; a retry never trips over a prior
    /// attempt's leftover dir (the old "deterministic" path made `gh repo clone`
    /// fail into a non-empty dir); and the unpredictable name defeats the classic
    /// world-writable-`/tmp` symlink pre-creation (TOCTOU) attack. The file write
    /// additionally uses create-new semantics (see [`Self::write_formula`]).
    fn fresh_workdir(name: &str, version: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "ossctl-homebrew-{name}-{version}-{}-{nanos}",
            std::process::id()
        ))
    }

    /// The branch the create path commits the new formula on.
    fn create_branch(name: &str, version: &str) -> String {
        format!("ossctl-homebrew-{name}-{version}")
    }

    /// The commit/PR title for a first formula.
    fn create_title(name: &str, version: &str) -> String {
        format!("{name} {version} (new formula)")
    }

    /// The ordered git/`gh` commands the create path runs (clone → branch → add →
    /// commit → push → PR), into the pre-computed `workdir`. The generated `.rb`
    /// is written to disk *between* the clone and the `add` (see
    /// [`Self::publish`]); these are only the process steps, shared by
    /// [`Self::dry_run`]'s preview and [`Self::publish`].
    ///
    /// `sha256_present` gates two things: a **draft** PR and a blocker in the body.
    /// When the source-tarball digest is not yet known (the coordinator threads
    /// `sha256: None` pre-tag), the generated formula carries only a `sha256`
    /// TODO and would fail `brew audit` / cannot install — so the PR is opened as a
    /// draft whose body states the one remaining manual step, rather than a
    /// mergeable-looking PR that is silently broken.
    fn create_commands(
        tap: &str,
        name: &str,
        version: &str,
        workdir: &str,
        sha256_present: bool,
    ) -> Vec<PlannedCommand> {
        let branch = Self::create_branch(name, version);
        let title = Self::create_title(name, version);
        let formula_rel = format!("Formula/{name}.rb");
        let body = if sha256_present {
            "Automated first-formula bootstrap by ossctl.".to_string()
        } else {
            "Automated first-formula bootstrap by ossctl.\n\n**Blocked:** the \
             `sha256` of the published release tarball is not yet known at cut \
             time (the tag archive does not exist until after publish). Fill in \
             the `sha256` once the tag is pushed, then mark this PR ready."
                .to_string()
        };
        let mut pr = vec![
            "pr".to_string(),
            "create".to_string(),
            "--repo".to_string(),
            tap.to_string(),
            "--head".to_string(),
            branch.clone(),
            "--title".to_string(),
            title.clone(),
            "--body".to_string(),
            body,
        ];
        if !sha256_present {
            pr.push("--draft".to_string());
        }
        vec![
            PlannedCommand::new("gh", &["repo", "clone", tap, workdir, "--", "--depth", "1"]),
            PlannedCommand::new("git", &["-C", workdir, "checkout", "-b", &branch]),
            PlannedCommand::new("git", &["-C", workdir, "add", &formula_rel]),
            // Set the commit identity explicitly (via `-c`): the freshly-cloned tap
            // inherits no `user.name`/`user.email`, so on a clean CI runner an
            // identity-less `git commit` fails with "Author identity unknown".
            PlannedCommand::new(
                "git",
                &[
                    "-C",
                    workdir,
                    "-c",
                    "user.name=ossctl",
                    "-c",
                    "user.email=ossctl@users.noreply.github.com",
                    "commit",
                    "-m",
                    &title,
                ],
            ),
            PlannedCommand::new(
                "git",
                &["-C", workdir, "push", "--set-upstream", "origin", &branch],
            ),
            PlannedCommand {
                program: "gh".to_string(),
                args: pr,
            },
        ]
    }

    /// Run the create path: generate the initial formula, clone the tap, write the
    /// file, commit it on a branch, and open a PR — all effects through the runner
    /// except the single filesystem write of the generated `.rb`.
    fn run_create(
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
        tap: &str,
    ) -> Result<PublishReceipt, AdapterError> {
        let tarball =
            ctx.artifacts
                .source_tarball
                .as_ref()
                .ok_or_else(|| AdapterError::Command {
                    command: "homebrew first-formula".into(),
                    code: None,
                    stderr: "cannot generate a first Homebrew formula without a resolvable GitHub \
                         source-tarball URL (no `origin` GitHub remote?)"
                        .into(),
                })?;
        let license = ctx
            .artifacts
            .homebrew
            .as_ref()
            .and_then(|h| h.license.as_deref());
        let homepage_slug = ctx.artifacts.repo_slug.as_deref();
        let formula = render_formula(
            &t.package,
            homepage_slug,
            &tarball.url,
            tarball.sha256.as_deref(),
            license,
        );

        // One workdir, computed once, used by both the clone and the write.
        let workdir = Self::fresh_workdir(&t.package, &t.version);
        let workdir_str = workdir.to_string_lossy().to_string();
        let commands = Self::create_commands(
            tap,
            &t.package,
            &t.version,
            &workdir_str,
            tarball.sha256.is_some(),
        );
        // 1. clone the tap.
        run_all(ctx, &commands[..1])?;
        // 2. write the generated formula into the checkout (create-new: refuses to
        //    overwrite a formula that already exists in the clone — the last-line
        //    guard against a probe/clone race or a mis-detected "absent").
        Self::write_formula(&workdir, &t.package, &formula)?;
        // 3. branch → add → commit → push → PR.
        let outputs = run_all(ctx, &commands[1..])?;

        // Record the PR URL `gh pr create` prints as the receipt's `remote_url`
        // (the field already existed — recording it is not a JSON-shape change).
        // `gh` can precede the URL with status lines, so take the last line that
        // looks like a URL rather than the whole stdout blob.
        let remote_url = outputs.last().and_then(|o| {
            o.stdout
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| line.starts_with("https://"))
                .map(str::to_string)
        });
        Ok(make_receipt(ctx, t, None, remote_url))
    }

    /// Write the generated formula to `<workdir>/Formula/<name>.rb`, creating the
    /// `Formula/` directory if the freshly-cloned tap does not carry it yet.
    ///
    /// This is the one direct-filesystem effect in the adapter — Homebrew has no
    /// "add a formula" CLI; a new formula *is* a committed file, so `run_all`
    /// (which only *runs processes*) cannot express it. It is deliberately scoped:
    /// it writes exactly one file into a private, unpredictable [`Self::fresh_workdir`]
    /// the create path just cloned into, and it uses **create-new** semantics
    /// (`O_EXCL`) so it never follows a symlink onto, or truncates, an existing
    /// file — which also fails loudly if the tap already carries the formula (a
    /// last-line guard against a mis-detected "absent" formula). A general
    /// filesystem port on `EffectCtx` is the cleaner long-term home (issue
    /// `homebrew-adapter-fs-port`); until then this is mapped to a distinct
    /// [`AdapterError::Filesystem`] so the effect is explicit, not hidden.
    fn write_formula(
        workdir: &std::path::Path,
        name: &str,
        formula: &str,
    ) -> Result<(), AdapterError> {
        let dir = workdir.join("Formula");
        std::fs::create_dir_all(&dir).map_err(|e| AdapterError::Filesystem {
            path: dir.to_string_lossy().to_string(),
            source: e.to_string(),
        })?;
        let path = dir.join(format!("{name}.rb"));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| AdapterError::Filesystem {
                path: path.to_string_lossy().to_string(),
                source: e.to_string(),
            })?;
        std::io::Write::write_all(&mut file, formula.as_bytes()).map_err(|e| {
            AdapterError::Filesystem {
                path: path.to_string_lossy().to_string(),
                source: e.to_string(),
            }
        })
    }
}

impl ReleaseAdapter for HomebrewAdapter {
    fn adapter(&self) -> Adapter {
        self.adapter
    }

    fn dry_run(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        let tarball = ctx.artifacts.source_tarball.as_ref();
        let (planned_commands, mut notes) = match self.resolve_path(ctx, t)? {
            FormulaPath::Create => {
                // `tap` is Some whenever resolve_path returned Create.
                let tap = self
                    .tap(ctx.artifacts.homebrew.as_ref())
                    .unwrap_or_default();
                let workdir = Self::fresh_workdir(&t.package, &t.version);
                let sha256_present = tarball.and_then(|tb| tb.sha256.as_deref()).is_some();
                (
                    Self::create_commands(
                        tap,
                        &t.package,
                        &t.version,
                        &workdir.to_string_lossy(),
                        sha256_present,
                    ),
                    vec![format!(
                        "create path: `{}` has no `{}.rb` yet — generating the initial \
                         source-build formula and opening a{} PR",
                        tap,
                        t.package,
                        if sha256_present { "" } else { " draft" }
                    )],
                )
            }
            FormulaPath::Bump => (
                vec![self.bump_command(tarball, &t.package)],
                vec!["bump path: the formula already exists — bumping its url/sha256".to_string()],
            ),
        };
        match tarball {
            Some(tb) => {
                let sha = tb
                    .sha256
                    .as_deref()
                    .unwrap_or("(computed by brew from --url)");
                notes.push(format!("url: {} ; sha256: {sha}", tb.url));
            }
            None => notes
                .push("source tarball url is resolved by the coordinator at cut time".to_string()),
        }
        Ok(DryRunReport {
            adapter: self.adapter,
            planned_commands,
            notes,
        })
    }

    fn build(
        &self,
        _ctx: &EffectCtx<'_>,
        _t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        // Homebrew has no build phase of its own — it repackages an existing
        // release artifact. Return an empty manifest rather than shelling out.
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts: vec![],
            notes: vec!["homebrew has no build phase (formula create/update only)".to_string()],
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // PER-TARGET IRREVERSIBLE (opens a formula create/bump PR).
        match self.resolve_path(ctx, t)? {
            FormulaPath::Create => {
                let tap = self
                    .tap(ctx.artifacts.homebrew.as_ref())
                    .expect("resolve_path returns Create only when a tap is configured");
                Self::run_create(ctx, t, tap)
            }
            FormulaPath::Bump => {
                let cmd = self.bump_command(ctx.artifacts.source_tarball.as_ref(), &t.package);
                run_all(ctx, &[cmd])?;
                Ok(make_receipt(ctx, t, None, None))
            }
        }
    }

    fn verify(
        &self,
        _ctx: &EffectCtx<'_>,
        _receipt: &PublishReceipt,
    ) -> Result<VerifyOutcome, AdapterError> {
        // A tap/core formula is not observable through RegistryQuery; report the
        // honest "cannot check" rather than a false Missing (ADR-0002 §1).
        Ok(VerifyOutcome::Unknown)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

/// Render a source-build Homebrew formula for `name` at `url`.
///
/// Produces the same shape as ossctl's own hand-written 0.1.0 formula: a cargo
/// source build (`depends_on "rust" => :build` + `cargo install`). The install
/// stanza is deliberately Rust-specific — the two consumers (`ossctl`,
/// `issuectl`) are cargo CLIs, and the issue this implements reproduces that
/// formula; a non-Rust source build is a documented follow-up.
///
/// `sha256`/`license` are optional: an absent `sha256` (the coordinator cannot
/// hash the pushed tag archive before it exists — see the coordinator's
/// `source_tarball` docs) emits a `TODO` placeholder the maintainer completes,
/// mirroring the 0.1.0 hand-fill; an absent `license` omits the stanza.
fn render_formula(
    name: &str,
    homepage_slug: Option<&str>,
    url: &str,
    sha256: Option<&str>,
    license: Option<&str>,
) -> String {
    let class = formula_class(name);
    // Every value interpolated into a Ruby double-quoted literal is escaped, so a
    // `"` / `\` in a contract-supplied value cannot break out of the string (or
    // inject Ruby). `name` reaches only `desc` and the `bin/"…"` test — the class
    // name is already alphanumeric-only.
    let name_lit = ruby_escape(name);
    let homepage = homepage_slug.map_or_else(
        || ruby_escape(url),
        |s| ruby_escape(&format!("https://github.com/{s}")),
    );
    let url_lit = ruby_escape(url);
    let sha_line = match sha256 {
        Some(sha) => format!("  sha256 \"{}\"", ruby_escape(sha)),
        None => "  # TODO: sha256 of the published release tarball \
                 (unavailable at cut time — fill in after the tag archive exists)"
            .to_string(),
    };
    let license_line = license
        .map(|l| format!("  license \"{}\"\n", ruby_escape(l)))
        .unwrap_or_default();
    format!(
        "class {class} < Formula\n\
         \x20 desc \"{name_lit}\"\n\
         \x20 homepage \"{homepage}\"\n\
         \x20 url \"{url_lit}\"\n\
         {sha_line}\n\
         {license_line}\
         \n\
         \x20 depends_on \"rust\" => :build\n\
         \n\
         \x20 def install\n\
         \x20   system \"cargo\", \"install\", *std_cargo_args\n\
         \x20 end\n\
         \n\
         \x20 test do\n\
         \x20   system bin/\"{name_lit}\", \"--version\"\n\
         \x20 end\n\
         end\n"
    )
}

/// Escape a value for inclusion in a Ruby double-quoted string literal:
/// backslashes first, then double quotes. Prevents a contract-supplied `"` or
/// `\` from terminating the literal or injecting Ruby.
fn ruby_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Homebrew's formula class name for `name`: alphanumeric runs capitalised and
/// concatenated (`my-tool` → `MyTool`, `ossctl` → `Ossctl`). A small, faithful
/// subset of Homebrew's `Formulary.class_s` — enough for the ordinary tap names
/// this generator targets.
///
/// A Ruby constant may not begin with a digit, so a leading-digit name is
/// prefixed with `X` (as Homebrew itself does: `2fa` → `X2fa`); a name that
/// reduces to nothing falls back to `Formula` so the output is always a legal
/// constant rather than a syntax error.
fn formula_class(name: &str) -> String {
    let mut out = String::new();
    for segment in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        return "Formula".to_string();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'X');
    }
    out
}
