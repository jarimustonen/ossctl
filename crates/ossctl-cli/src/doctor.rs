//! `ossctl doctor` — read-only self-diagnostic (`AGENTS-AI-FIRST-CLI.md` §18).
//!
//! Runs the tool's internal self-check and reports each finding so an agent can
//! answer "is the tool itself healthy?" in one call. Read-only by default; the
//! corrective twin `--fix` applies the safe subset of suggestions.
//!
//! Scaffold scope: the check *harness* is real (per-check `id`/`status`/
//! `message`/`fix_suggestion`, §18 summary + exit semantics, `--fix`/`--dry-run`
//! plumbing). The concrete check set is a compiling skeleton — schema, deps,
//! skill-sync, config, and data-integrity checks fill in as those subsystems
//! land in later units.

use std::process::ExitCode;

use clap::Args;
use serde::Serialize;

use crate::error::CliError;
use crate::output::OutputFormat;

/// Arguments for `ossctl doctor`.
#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Apply the safe subset of `fix_suggestion`s after running the checks.
    /// Opt-in per invocation; never the default (§18).
    #[arg(long)]
    pub fix: bool,
    /// With `--fix`, print the planned fixes and apply nothing (§11).
    #[arg(long)]
    pub dry_run: bool,
}

/// A check's outcome (§18).
///
/// The scaffold's only check is always `Ok`, so `Warn`/`Fail` are not yet
/// *constructed* — but they are the §18 contract the render/summary/exit-code
/// paths already handle, and the checks added by later units produce them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Serialize)]
struct CheckResult {
    id: &'static str,
    status: Status,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix_suggestion: Option<String>,
}

#[derive(Debug, Serialize)]
struct Summary {
    ok: usize,
    warn: usize,
    fail: usize,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    checks: Vec<CheckResult>,
    summary: Summary,
}

/// Entry point. Returns an [`ExitCode`] directly because §18 exit semantics —
/// exit 1 on any `fail` *without* an error envelope (the report on stdout is
/// the answer) — do not map onto the shared error path.
pub fn run(args: &DoctorArgs, format: OutputFormat) -> ExitCode {
    match run_inner(args, format) {
        Ok(code) => code,
        Err(e) => {
            e.emit();
            ExitCode::from(e.kind as u8)
        }
    }
}

fn run_inner(args: &DoctorArgs, format: OutputFormat) -> Result<ExitCode, CliError> {
    if args.dry_run && !args.fix {
        return Err(CliError::user(
            "invalid_arguments",
            "--dry-run only applies with --fix (doctor is read-only by default)",
        ));
    }

    let checks = run_checks();
    let summary = summarize(&checks);
    let any_fail = summary.fail > 0;
    let report = DoctorReport { checks, summary };

    match format {
        OutputFormat::Json => crate::output::emit_json(&report, &[])?,
        OutputFormat::Text => render_text(&report)?,
    }

    // `--fix` has nothing to apply yet: the scaffold's checks carry no
    // auto-fixable suggestions. Report that plainly rather than silently
    // no-op'ing, so the `--fix` path is honest when checks grow fixable. Only
    // in text mode — a JSON caller's stderr is the fatal-only error channel
    // (§10); fix outcomes belong in the stdout report once they are real.
    if args.fix && format == OutputFormat::Text {
        let verb = if args.dry_run {
            "would apply"
        } else {
            "applied"
        };
        eprintln!("doctor --fix: {verb} 0 fixes (no auto-fixable checks yet)");
    }

    // §18: exit 1 on any fail, else 0 — warnings never flip the code. This 1 is
    // a distinct axis from §2's user-error exit 1: a `fail` is the *tool*
    // finding a problem, not a bad invocation. Written as a bare `1` (not
    // `ExitKind::User`) so that semantic distinction is not implied.
    Ok(if any_fail {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// The scaffold check set. A single always-`ok` self-check that proves the
/// harness end-to-end; real checks (schema, deps, skill-sync, config, data
/// integrity) are added by the units that introduce those subsystems.
fn run_checks() -> Vec<CheckResult> {
    vec![CheckResult {
        id: "binary.self",
        status: Status::Ok,
        message: format!("ossctl {} responding", env!("CARGO_PKG_VERSION")),
        fix_suggestion: None,
    }]
}

fn summarize(checks: &[CheckResult]) -> Summary {
    let mut s = Summary {
        ok: 0,
        warn: 0,
        fail: 0,
    };
    for c in checks {
        match c.status {
            Status::Ok => s.ok += 1,
            Status::Warn => s.warn += 1,
            Status::Fail => s.fail += 1,
        }
    }
    s
}

fn render_text(report: &DoctorReport) -> Result<(), CliError> {
    for c in &report.checks {
        let tag = match c.status {
            Status::Ok => "OK",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
        };
        crate::output::stdoutln!("{tag:<4} {}  {}", c.id, c.message);
        if let Some(fix) = &c.fix_suggestion {
            crate::output::stdoutln!("       fix: {fix}");
        }
    }
    crate::output::stdoutln!(
        "summary: {} ok, {} warn, {} fail",
        report.summary.ok,
        report.summary.warn,
        report.summary.fail
    );
    Ok(())
}
