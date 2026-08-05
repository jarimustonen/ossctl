//! Unit tests for the `dist generate` handler (flow + error paths), driving the
//! deterministic core through a recording [`CommandRunner`] fake so no real
//! `dist` tool is required.

use std::cell::RefCell;
use std::io;

use ossctl_core::ports::{CommandOutput, CommandRunner};

use super::*;

/// A `CommandRunner` that records its invocations and returns a scripted result,
/// so the `dist generate` step is exercised without cargo-dist installed.
struct RecordingRunner {
    calls: RefCell<Vec<String>>,
    behavior: Behavior,
}

enum Behavior {
    /// `dist generate` exits 0 and produces the workflow file (like the real tool).
    Ok,
    /// `dist generate` exits 0 but writes NO workflow (a stale/mismatched tool).
    OkNoWorkflow,
    /// The `dist` tool is not installed.
    NotFound,
    /// `dist generate` runs but exits non-zero.
    NonZero,
}

impl RecordingRunner {
    fn new(behavior: Behavior) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            behavior,
        }
    }
}

impl CommandRunner for RecordingRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        cwd: &std::path::Path,
    ) -> io::Result<CommandOutput> {
        self.calls
            .borrow_mut()
            .push(format!("{program} {}", args.join(" ")));
        let ok = || {
            Ok(CommandOutput {
                status: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        };
        match self.behavior {
            Behavior::Ok => {
                // Emulate the real tool writing the workflow file.
                let wf = cwd.join(".github/workflows/release.yml");
                std::fs::create_dir_all(wf.parent().unwrap()).unwrap();
                std::fs::write(&wf, "# generated\n").unwrap();
                ok()
            }
            Behavior::OkNoWorkflow => ok(),
            Behavior::NotFound => Err(io::Error::new(io::ErrorKind::NotFound, "dist not found")),
            Behavior::NonZero => Ok(CommandOutput {
                status: Some(1),
                stdout: String::new(),
                stderr: "cargo-dist: no distributable packages".to_string(),
            }),
        }
    }
}

/// Write an `OSS-RELEASE.md` with the given `distribution:` YAML block (or none).
fn write_contract(dir: &std::path::Path, distribution: Option<&str>) {
    let mut doc = String::from(
        "---\n\
         schema_version: 1\n\
         status: approved\n\
         maturity: mvp\n\
         ecosystems: [rust]\n\
         versioning: semver\n\
         changelog:\n\
         \x20 mode: fragment\n\
         \x20 source: issuectl-trailers\n\
         conventional_commits: false\n\
         release:\n\
         \x20 model: gated\n\
         \x20 layout: single\n\
         contribution_provenance: none\n\
         provenance_level: keyless\n\
         dependency_bot: dependabot\n\
         health_badges: [ci]\n\
         license: MIT\n\
         docs_site: none\n",
    );
    if let Some(block) = distribution {
        doc.push_str(block);
    }
    doc.push_str("---\n\n# Test contract\n");
    std::fs::write(dir.join("OSS-RELEASE.md"), doc).unwrap();
}

fn args(dir: &std::path::Path, force: bool, no_workflow: bool) -> GenerateArgs {
    GenerateArgs {
        repo_root: Some(dir.to_path_buf()),
        require_approved: false,
        force,
        no_workflow,
    }
}

#[test]
fn generates_config_and_invokes_dist() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: cargo-dist\n  installers: [shell, powershell]\n"),
    );
    let runner = RecordingRunner::new(Behavior::Ok);

    generate(&args(dir.path(), false, false), OutputFormat::Text, &runner)
        .expect("generate succeeds");

    // The config was written with the reference shape.
    let toml = std::fs::read_to_string(dir.path().join("dist-workspace.toml")).unwrap();
    assert!(toml.contains("[dist]"), "{toml}");
    assert!(toml.contains("pr-run-mode = \"skip\""), "{toml}");
    assert!(
        toml.contains("aarch64-unknown-linux-musl"),
        "cross-platform default: {toml}"
    );

    // `dist generate` was invoked exactly once.
    let calls = runner.calls.borrow();
    assert_eq!(calls.len(), 1, "one dist invocation: {calls:?}");
    assert_eq!(calls[0], "dist generate");
}

#[test]
fn no_workflow_skips_the_dist_invocation() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: cargo-dist\n  installers: [shell]\n"),
    );
    let runner = RecordingRunner::new(Behavior::NotFound); // would error if called

    generate(&args(dir.path(), false, true), OutputFormat::Text, &runner)
        .expect("generate succeeds without the tool");

    assert!(
        dir.path().join("dist-workspace.toml").exists(),
        "config still written"
    );
    assert!(runner.calls.borrow().is_empty(), "dist must NOT be invoked");
}

#[test]
fn missing_distribution_block_is_user_error() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(dir.path(), None);
    let runner = RecordingRunner::new(Behavior::Ok);

    let err = generate(&args(dir.path(), false, false), OutputFormat::Text, &runner).unwrap_err();
    assert_eq!(err.code, "no_distribution");
    assert!(matches!(err.kind, crate::error::ExitKind::User));
    assert!(
        !dir.path().join("dist-workspace.toml").exists(),
        "nothing written"
    );
}

#[test]
fn goreleaser_adapter_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: goreleaser\n  installers: [shell]\n"),
    );
    let runner = RecordingRunner::new(Behavior::Ok);

    let err = generate(&args(dir.path(), false, false), OutputFormat::Text, &runner).unwrap_err();
    assert_eq!(err.code, "unsupported_distribution_adapter");
    assert_eq!(err.invalid_value.as_deref(), Some("goreleaser"));
    assert!(
        !dir.path().join("dist-workspace.toml").exists(),
        "nothing written"
    );
}

#[test]
fn existing_config_is_not_clobbered_without_force() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: cargo-dist\n  installers: [shell]\n"),
    );
    std::fs::write(dir.path().join("dist-workspace.toml"), "# hand-tuned\n").unwrap();
    let runner = RecordingRunner::new(Behavior::Ok);

    let err = generate(&args(dir.path(), false, false), OutputFormat::Text, &runner).unwrap_err();
    assert_eq!(err.code, "dist_config_exists");
    // The existing content is preserved.
    let kept = std::fs::read_to_string(dir.path().join("dist-workspace.toml")).unwrap();
    assert_eq!(kept, "# hand-tuned\n", "must not overwrite without --force");
    assert!(
        runner.calls.borrow().is_empty(),
        "dist not invoked on refusal"
    );
}

#[test]
fn force_overwrites_existing_config() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: cargo-dist\n  installers: [shell]\n"),
    );
    std::fs::write(dir.path().join("dist-workspace.toml"), "# hand-tuned\n").unwrap();
    let runner = RecordingRunner::new(Behavior::Ok);

    generate(&args(dir.path(), true, false), OutputFormat::Text, &runner).expect("force succeeds");
    let written = std::fs::read_to_string(dir.path().join("dist-workspace.toml")).unwrap();
    assert!(
        written.contains("[dist]"),
        "force replaced the config: {written}"
    );
}

#[test]
fn missing_dist_tool_is_a_clear_system_error() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: cargo-dist\n  installers: [shell]\n"),
    );
    let runner = RecordingRunner::new(Behavior::NotFound);

    let err = generate(&args(dir.path(), false, false), OutputFormat::Text, &runner).unwrap_err();
    assert_eq!(err.code, "dist_tool_missing");
    assert!(matches!(err.kind, crate::error::ExitKind::System));
    // The config was still written (only the workflow step failed).
    assert!(
        dir.path().join("dist-workspace.toml").exists(),
        "config written before dist ran"
    );
}

#[test]
fn nonzero_dist_exit_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: cargo-dist\n  installers: [shell]\n"),
    );
    let runner = RecordingRunner::new(Behavior::NonZero);

    let err = generate(&args(dir.path(), false, false), OutputFormat::Text, &runner).unwrap_err();
    assert_eq!(err.code, "dist_generate_failed");
    assert!(
        err.message.contains("no distributable packages"),
        "surfaces stderr: {}",
        err.message
    );
}

#[test]
fn rerun_with_identical_config_is_idempotent_without_force() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: cargo-dist\n  installers: [shell]\n"),
    );
    // First run writes the config but skips the workflow (as if the tool was absent).
    let r1 = RecordingRunner::new(Behavior::NotFound);
    generate(&args(dir.path(), false, true), OutputFormat::Text, &r1).expect("config written");

    // Second run, tool now available, NO --force: the on-disk config is byte-identical,
    // so it must NOT refuse with dist_config_exists — it proceeds to the workflow.
    let r2 = RecordingRunner::new(Behavior::Ok);
    generate(&args(dir.path(), false, false), OutputFormat::Text, &r2)
        .expect("idempotent re-run must not require --force");
    assert_eq!(r2.calls.borrow().len(), 1, "workflow step ran on re-run");
    assert!(dir.path().join(".github/workflows/release.yml").exists());
}

#[test]
fn zero_exit_without_a_workflow_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    write_contract(
        dir.path(),
        Some("distribution:\n  adapter: cargo-dist\n  installers: [shell]\n"),
    );
    // The tool exits 0 but produces no workflow (a stale/mismatched cargo-dist).
    let runner = RecordingRunner::new(Behavior::OkNoWorkflow);

    let err = generate(&args(dir.path(), false, false), OutputFormat::Text, &runner).unwrap_err();
    assert_eq!(err.code, "dist_workflow_missing");
    assert!(matches!(err.kind, crate::error::ExitKind::System));
}

#[test]
fn require_approved_refuses_a_draft() {
    let dir = tempfile::tempdir().unwrap();
    // Same contract but draft status.
    let doc = std::fs::read_to_string({
        write_contract(
            dir.path(),
            Some("distribution:\n  adapter: cargo-dist\n  installers: [shell]\n"),
        );
        dir.path().join("OSS-RELEASE.md")
    })
    .unwrap()
    .replace("status: approved", "status: draft");
    std::fs::write(dir.path().join("OSS-RELEASE.md"), doc).unwrap();
    let runner = RecordingRunner::new(Behavior::Ok);

    let mut a = args(dir.path(), false, false);
    a.require_approved = true;
    let err = generate(&a, OutputFormat::Text, &runner).unwrap_err();
    assert_eq!(err.code, "not_approved");
}
