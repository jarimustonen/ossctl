//! Read-only external-tool preflight for release execution.
//!
//! A fresh cut runs this after authenticating the sealed plan and before creating
//! its journal. Resume runs it against the journal projection, so only tools needed
//! by an incomplete phase are checked. The checks use [`CommandRunner`] exclusively:
//! tests control the observed PATH and production never installs a tool implicitly.

use std::collections::BTreeSet;
use std::path::Path;

use crate::contract::schema::Adapter;
use crate::ports::CommandRunner;
use crate::protocol::journal::{Phase, PhaseOutcome, RunState};
use crate::protocol::plan::ReleasePlan;
use crate::release::journal_target_ids;

/// The cargo-dist configuration file whose pin governs the local `dist` binary.
pub const DIST_WORKSPACE_FILENAME: &str = "dist-workspace.toml";

/// The class of release dependency failure, used by CLI callers to route remediation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyErrorKind {
    /// A required host executable is missing, broken, or the wrong version.
    Host,
    /// The sealed repository configuration does not declare a valid dependency.
    Configuration,
}

/// A release dependency could not be validated before execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyError {
    /// Whether the host or the sealed repository configuration needs repair.
    pub kind: DependencyErrorKind,
    /// Executable (or executable group) that failed validation.
    pub executable: String,
    /// Exact required version, when the release configuration pins one.
    pub required_version: Option<String>,
    /// What the read-only observer found.
    pub found: Option<String>,
    /// Operator-facing explanation and retry instructions.
    pub message: String,
}

impl std::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DependencyError {}

/// Validate every external executable needed by the sealed plan's incomplete
/// phases. `state == None` means a fresh cut (all phases remain).
///
/// `dist_workspace` must contain the sealed commit's `dist-workspace.toml` when
/// cargo-dist remains required. Keeping the bytes explicit lets the caller read
/// them from the authenticated commit rather than from a potentially drifted live
/// checkout.
pub fn check(
    plan: &ReleasePlan,
    state: Option<&RunState>,
    runner: &dyn CommandRunner,
    repo_root: &Path,
    dist_workspace: Option<&str>,
) -> Result<(), DependencyError> {
    let requirements = required_executables(plan, state);
    for executable in requirements
        .executables
        .iter()
        .filter(|executable| **executable != "dist")
    {
        probe(executable, runner, repo_root)?;
    }
    if requirements.sha256 {
        probe_sha256(runner, repo_root)?;
    }
    if requirements.executables.contains("dist") {
        let required = pinned_cargo_dist_version(dist_workspace)?;
        check_dist_version(&required, runner, repo_root)?;
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Requirements {
    executables: BTreeSet<&'static str>,
    sha256: bool,
}

/// Whether an incomplete build barrier will invoke cargo-dist.
#[must_use]
pub fn cargo_dist_required(plan: &ReleasePlan, state: Option<&RunState>) -> bool {
    if phase_completed(state, Phase::Build) {
        return false;
    }
    plan.targets
        .iter()
        .zip(journal_target_ids(&plan.targets))
        .any(|(target, id)| {
            target.adapter == Adapter::CargoDist && !target_completed(state, Phase::Build, &id)
        })
}

#[allow(clippy::too_many_lines)] // One exhaustive adapter-to-executable matrix is easier to audit.
fn required_executables(plan: &ReleasePlan, state: Option<&RunState>) -> Requirements {
    let remains = |phase| !phase_completed(state, phase);
    let mut required = Requirements::default();

    // Every non-terminal execution enters through a sealed git checkout; tagging,
    // bump commits, and final branch advancement use git as well.
    required.executables.insert("git");

    if plan.bump.is_some() && remains(Phase::Bump) {
        required.executables.insert("cargo");
        if plan
            .bump
            .as_ref()
            .and_then(|bump| bump.bump_hook.as_ref())
            .is_some()
        {
            required.executables.insert("sh");
        }
    }

    for (target, id) in plan.targets.iter().zip(journal_target_ids(&plan.targets)) {
        let dry_run_remains =
            remains(Phase::DryRun) && !target_completed(state, Phase::DryRun, &id);
        let build_remains = remains(Phase::Build) && !target_completed(state, Phase::Build, &id);
        let publish_remains =
            remains(Phase::Publish) && !target_completed(state, Phase::Publish, &id);
        let dist_remains = remains(Phase::Dist) && !target_completed(state, Phase::Dist, &id);
        let verify_remains = remains(Phase::Verify) && !target_completed(state, Phase::Verify, &id);
        match target.adapter {
            Adapter::CargoPublish | Adapter::CargoPublishCi => {
                if dry_run_remains
                    || build_remains
                    || (target.adapter == Adapter::CargoPublish && publish_remains)
                {
                    required.executables.insert("cargo");
                }
                if target.adapter == Adapter::CargoPublish && publish_remains {
                    // An already-published crate is skipped only after packaging and
                    // SHA-256-authenticating the intended artifact.
                    required.sha256 = true;
                }
                if target.adapter == Adapter::CargoPublishCi && verify_remains {
                    required.executables.insert("gh");
                }
            }
            Adapter::CargoDist => {
                // Cargo-dist's dry-run only renders `dist plan`; `dist build` is
                // the first phase that actually spawns the pinned executable.
                if cargo_dist_required(plan, state) {
                    required.executables.insert("dist");
                }
                if verify_remains {
                    required.executables.insert("gh");
                }
            }
            Adapter::Changesets => {
                if build_remains {
                    required.executables.insert("npm");
                }
                if publish_remains {
                    required.executables.insert("changeset");
                }
            }
            Adapter::ReleasePlease => {
                if build_remains {
                    required.executables.insert("npm");
                }
            }
            Adapter::NpmPublish => {
                if build_remains || publish_remains {
                    required.executables.insert("npm");
                }
            }
            Adapter::GhActionPypiPublish => {
                if build_remains {
                    required.executables.insert("python");
                }
            }
            Adapter::Twine => {
                if build_remains {
                    required.executables.insert("python");
                }
                if publish_remains {
                    required.executables.insert("twine");
                }
            }
            Adapter::Goreleaser => {
                if build_remains || publish_remains {
                    required.executables.insert("goreleaser");
                }
            }
            Adapter::HomebrewTap => {
                if dry_run_remains || dist_remains {
                    required.executables.insert("gh");
                }
                if dist_remains {
                    required.executables.insert("git");
                    required.executables.insert("curl");
                    required.sha256 = true;
                }
            }
            Adapter::HomebrewCore => {
                if dry_run_remains {
                    required.executables.insert("gh");
                }
                if dist_remains {
                    required.executables.insert("brew");
                    required.executables.insert("curl");
                    required.sha256 = true;
                }
            }
            Adapter::Manual => {
                if publish_remains || verify_remains {
                    required.executables.insert("gh");
                }
            }
        }
    }

    if remains(Phase::Tag)
        && !plan.targets.is_empty()
        && !plan
            .targets
            .iter()
            .any(|target| target.adapter == Adapter::CargoDist)
    {
        required.executables.insert("gh");
    }

    // Registry observers are in-process. GitHub Release observers use `gh`; the
    // CargoDist and Manual arms above account for those precise target types.
    required
}

fn phase_completed(state: Option<&RunState>, phase: Phase) -> bool {
    state.is_some_and(|state| {
        state
            .phases
            .iter()
            .any(|record| record.phase == phase && record.outcome == PhaseOutcome::Ok)
    })
}

fn target_completed(state: Option<&RunState>, phase: Phase, target: &str) -> bool {
    state.is_some_and(|state| match phase {
        Phase::DryRun => state.dry_run.contains(target),
        Phase::Build => state.built.contains(target),
        Phase::Publish | Phase::Dist => {
            state.published.contains_key(target) || state.delegated.contains(target)
        }
        Phase::Verify => {
            state.verified.get(target) == Some(&crate::protocol::release::VerifyOutcome::Matches)
        }
        Phase::Bump | Phase::Tag | Phase::AdvanceBranch => false,
    })
}

fn probe(
    executable: &str,
    runner: &dyn CommandRunner,
    repo_root: &Path,
) -> Result<String, DependencyError> {
    let args: &[&str] = if executable == "sh" {
        &["-c", "exit 0"]
    } else {
        &["--version"]
    };
    let output = runner
        .run(executable, args, repo_root)
        .map_err(|error| missing(executable, &error.to_string()))?;
    if output.status != Some(0) {
        let detail = if output.stderr.trim().is_empty() {
            output.stdout.trim()
        } else {
            output.stderr.trim()
        };
        return Err(missing(
            executable,
            &format!("version probe exited {:?}: {detail}", output.status),
        ));
    }
    let observed = if output.stdout.trim().is_empty() {
        output.stderr.trim()
    } else {
        output.stdout.trim()
    };
    Ok(observed.to_string())
}

fn missing(executable: &str, detail: &str) -> DependencyError {
    DependencyError {
        kind: DependencyErrorKind::Host,
        executable: executable.to_string(),
        required_version: None,
        found: Some(detail.to_string()),
        message: format!(
            "required release executable `{executable}` is unavailable ({detail}). Install it or place it on PATH, then retry the same `shipshape release cut`/`resume` command; shipshape does not install release tools automatically"
        ),
    }
}

fn probe_sha256(runner: &dyn CommandRunner, repo_root: &Path) -> Result<(), DependencyError> {
    // Every approved release root has this contract, regardless of ecosystem.
    const INPUT: &str = "OSS-RELEASE.md";
    let candidates = [
        ("sha256sum", vec!["--", INPUT]),
        ("shasum", vec!["-a", "256", "--", INPUT]),
    ];
    let mut failures = Vec::new();
    for (executable, args) in candidates {
        match runner.run(executable, &args, repo_root) {
            Ok(output)
                if output.status == Some(0)
                    && output.stdout.split_whitespace().any(|token| {
                        token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
                    }) =>
            {
                return Ok(());
            }
            Ok(output) => failures.push(format!(
                "`{executable}` hash probe exited {:?}: {}",
                output.status,
                if output.stderr.trim().is_empty() {
                    output.stdout.trim()
                } else {
                    output.stderr.trim()
                }
            )),
            Err(error) => failures.push(format!("cannot run `{executable}`: {error}")),
        }
    }
    Err(DependencyError {
        kind: DependencyErrorKind::Host,
        executable: "sha256sum or shasum".to_string(),
        required_version: None,
        found: Some(failures.join("; ")),
        message: format!(
            "a SHA-256 executable is required for the remaining release phases, but neither sha256sum nor shasum can hash files. Install one and place it on PATH, then retry the same `shipshape release cut`/`resume` command; shipshape does not install release tools automatically ({})",
            failures.join("; ")
        ),
    })
}

fn pinned_cargo_dist_version(contents: Option<&str>) -> Result<String, DependencyError> {
    let absent = || {
        DependencyError {
        kind: DependencyErrorKind::Configuration,
        executable: "dist".to_string(),
        required_version: None,
        found: None,
        message: format!(
            "the sealed plan requires cargo-dist, but `{DIST_WORKSPACE_FILENAME}` does not provide an exact `[dist].cargo-dist-version` pin. Generate or fix the cargo-dist configuration, re-plan, and retry; shipshape does not install cargo-dist automatically"
        ),
    }
    };
    let document: toml::Value = toml::from_str(contents.ok_or_else(absent)?).map_err(|error| {
        let mut result = absent();
        result.message = format!(
            "cannot parse the sealed `{DIST_WORKSPACE_FILENAME}` while validating cargo-dist: {error}. Fix it, re-plan, and retry"
        );
        result
    })?;
    let version = document
        .get("dist")
        .and_then(|dist| dist.get("cargo-dist-version"))
        .and_then(toml::Value::as_str)
        .filter(|value| is_exact_version(value))
        .ok_or_else(absent)?;
    Ok(version.to_string())
}

fn is_exact_version(value: &str) -> bool {
    semver::Version::parse(value).is_ok()
}

fn check_dist_version(
    required: &str,
    runner: &dyn CommandRunner,
    repo_root: &Path,
) -> Result<(), DependencyError> {
    let observed = probe("dist", runner, repo_root).map_err(|mut error| {
        error.required_version = Some(required.to_string());
        error.message = format!(
            "required release executable `dist` is unavailable, but `{DIST_WORKSPACE_FILENAME}` pins cargo-dist {required}. Install that exact version with `cargo install cargo-dist --version {required} --locked` (or place a verified `dist` {required} binary on PATH), then retry the same command; shipshape does not install cargo-dist automatically"
        );
        error
    })?;
    if reported_dist_version(&observed) == Some(required) {
        return Ok(());
    }
    Err(DependencyError {
        kind: DependencyErrorKind::Host,
        executable: "dist".to_string(),
        required_version: Some(required.to_string()),
        found: Some(observed.clone()),
        message: format!(
            "release requires cargo-dist {required} as pinned by `{DIST_WORKSPACE_FILENAME}`, but `dist --version` reported {observed:?}. Install the exact version with `cargo install cargo-dist --version {required} --locked` (or place a verified `dist` {required} binary earlier on PATH), then retry the same command; shipshape does not install cargo-dist automatically"
        ),
    })
}

fn reported_dist_version(output: &str) -> Option<&str> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        match (fields.next()?, fields.next()?) {
            ("cargo-dist" | "dist", version) => Some(version),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;

    use super::*;
    use crate::contract::schema::{Ecosystem, Registry};
    use crate::ports::CommandOutput;
    use crate::protocol::journal::PhaseRecord;
    use crate::protocol::plan::{PlanPhase, PlanTarget};

    struct FakeRunner {
        versions: std::collections::HashMap<String, io::Result<CommandOutput>>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeRunner {
        fn with_dist(output: io::Result<CommandOutput>) -> Self {
            let mut versions = std::collections::HashMap::new();
            versions.insert("dist".into(), output);
            Self {
                versions,
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
            self.calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")).trim().to_string());
            self.versions.get(program).map_or_else(
                || {
                    Ok(CommandOutput {
                        status: Some(0),
                        stdout: if matches!(program, "sha256sum" | "shasum") {
                            format!("{}  Cargo.toml", "a".repeat(64))
                        } else {
                            format!("{program} 1.0.0")
                        },
                        stderr: String::new(),
                    })
                },
                |result| match result {
                    Ok(output) => Ok(output.clone()),
                    Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
                },
            )
        }
    }

    fn cargo_dist_plan() -> ReleasePlan {
        ReleasePlan {
            plan_id: "plan".into(),
            contract_schema_version: 1,
            head_sha: "abc".into(),
            version: "1.2.3".into(),
            targets: vec![PlanTarget {
                ecosystem: Ecosystem::Rust,
                package: Some("tool".into()),
                registry: Registry::GhReleases,
                adapter: Adapter::CargoDist,
            }],
            phases: PlanPhase::sequence().to_vec(),
            bump: None,
            homebrew_tap: None,
            license: None,
            description: None,
            homebrew_platforms: vec![],
        }
    }

    fn cargo_publish_plan() -> ReleasePlan {
        let mut plan = cargo_dist_plan();
        plan.targets[0].registry = Registry::CratesIo;
        plan.targets[0].adapter = Adapter::CargoPublish;
        plan
    }

    fn output(version: &str) -> CommandOutput {
        CommandOutput {
            status: Some(0),
            stdout: format!("cargo-dist {version}\n"),
            stderr: String::new(),
        }
    }

    #[test]
    fn cargo_publish_requires_hashing_until_its_publish_is_recorded() {
        let plan = cargo_publish_plan();
        let runner = FakeRunner::with_dist(Ok(output("0.32.0")));
        check(&plan, None, &runner, Path::new("/repo"), None).unwrap();
        assert!(runner
            .calls
            .borrow()
            .iter()
            .any(|call| call == "sha256sum -- OSS-RELEASE.md"));

        let runner = FakeRunner::with_dist(Ok(output("0.32.0")));
        let mut state = state_with_completed(&[Phase::DryRun, Phase::Build]);
        let target = journal_target_ids(&plan.targets).remove(0);
        state.published.insert(
            target,
            crate::protocol::journal::PublishReceipt {
                ecosystem: "cargo".into(),
                package: Some("tool".into()),
                version: "1.2.3".into(),
                registry_url: None,
                digest: None,
            },
        );
        check(&plan, Some(&state), &runner, Path::new("/repo"), None).unwrap();
        assert!(!runner
            .calls
            .borrow()
            .iter()
            .any(|call| call.starts_with("sha256sum ") || call.starts_with("shasum ")));
    }

    #[test]
    fn sha_probe_falls_back_to_shasum_using_real_hash_invocations() {
        let mut versions = std::collections::HashMap::new();
        versions.insert(
            "sha256sum".into(),
            Ok(CommandOutput {
                status: Some(1),
                stdout: String::new(),
                stderr: "unsupported --version".into(),
            }),
        );
        versions.insert(
            "shasum".into(),
            Ok(CommandOutput {
                status: Some(0),
                stdout: format!("{}  OSS-RELEASE.md", "b".repeat(64)),
                stderr: String::new(),
            }),
        );
        let runner = FakeRunner {
            versions,
            calls: RefCell::new(Vec::new()),
        };

        check(
            &cargo_publish_plan(),
            None,
            &runner,
            Path::new("/repo"),
            None,
        )
        .unwrap();
        assert_eq!(
            runner
                .calls
                .borrow()
                .iter()
                .filter(|call| call.starts_with("sha256sum ") || call.starts_with("shasum "))
                .cloned()
                .collect::<Vec<_>>(),
            [
                "sha256sum -- OSS-RELEASE.md",
                "shasum -a 256 -- OSS-RELEASE.md"
            ]
        );
    }

    #[test]
    fn missing_sha_tools_are_refused_for_cargo_publish() {
        let mut versions = std::collections::HashMap::new();
        for executable in ["sha256sum", "shasum"] {
            versions.insert(
                executable.into(),
                Err(io::Error::new(io::ErrorKind::NotFound, "not found")),
            );
        }
        let runner = FakeRunner {
            versions,
            calls: RefCell::new(Vec::new()),
        };
        let error = check(
            &cargo_publish_plan(),
            None,
            &runner,
            Path::new("/repo"),
            None,
        )
        .unwrap_err();
        assert_eq!(error.executable, "sha256sum or shasum");
        assert_eq!(error.kind, DependencyErrorKind::Host);
    }

    #[test]
    fn missing_cargo_dist_names_the_pin_and_retry_without_installing() {
        let runner =
            FakeRunner::with_dist(Err(io::Error::new(io::ErrorKind::NotFound, "not found")));
        let error = check(
            &cargo_dist_plan(),
            None,
            &runner,
            Path::new("/repo"),
            Some("[dist]\ncargo-dist-version = \"0.32.0\"\n"),
        )
        .unwrap_err();
        assert_eq!(error.executable, "dist");
        assert_eq!(error.required_version.as_deref(), Some("0.32.0"));
        assert!(error
            .message
            .contains("cargo install cargo-dist --version 0.32.0 --locked"));
        assert!(error.message.contains("does not install"));
    }

    #[test]
    fn wrong_cargo_dist_version_is_refused() {
        let runner = FakeRunner::with_dist(Ok(output("0.31.0")));
        let error = check(
            &cargo_dist_plan(),
            None,
            &runner,
            Path::new("/repo"),
            Some("[dist]\ncargo-dist-version = \"0.32.0\"\n"),
        )
        .unwrap_err();
        assert_eq!(error.required_version.as_deref(), Some("0.32.0"));
        assert_eq!(error.found.as_deref(), Some("cargo-dist 0.31.0"));
    }

    #[test]
    fn matching_cargo_dist_version_passes() {
        let runner = FakeRunner::with_dist(Ok(output("0.32.0")));
        check(
            &cargo_dist_plan(),
            None,
            &runner,
            Path::new("/repo"),
            Some("[dist]\ncargo-dist-version = \"0.32.0\"\n"),
        )
        .unwrap();
    }

    #[test]
    fn cargo_dist_version_must_be_the_reported_field() {
        let runner = FakeRunner::with_dist(Ok(CommandOutput {
            status: Some(0),
            stdout: "cargo-dist 0.31.0\nwarning: expected 0.32.0\n".into(),
            stderr: String::new(),
        }));
        let error = check(
            &cargo_dist_plan(),
            None,
            &runner,
            Path::new("/repo"),
            Some("[dist]\ncargo-dist-version = \"0.32.0\"\n"),
        )
        .unwrap_err();
        assert_eq!(
            error.found.as_deref(),
            Some("cargo-dist 0.31.0\nwarning: expected 0.32.0")
        );
    }

    #[test]
    fn malformed_semver_is_not_an_exact_pin() {
        for malformed in ["1.2.3-", "1.2.3+", "1.2.3-alpha..1", "1.2.3-01"] {
            assert!(!is_exact_version(malformed), "accepted {malformed}");
        }
        assert!(is_exact_version("1.2.3-rc.1+build.4"));
    }

    #[test]
    fn resume_rechecks_dist_while_build_remains() {
        let runner = FakeRunner::with_dist(Ok(output("0.32.0")));
        let state = state_with_completed(&[Phase::DryRun]);
        check(
            &cargo_dist_plan(),
            Some(&state),
            &runner,
            Path::new("/repo"),
            Some("[dist]\ncargo-dist-version = \"0.32.0\"\n"),
        )
        .unwrap();
        assert!(runner
            .calls
            .borrow()
            .iter()
            .any(|call| call == "dist --version"));
    }

    #[test]
    fn partial_build_resume_skips_an_already_built_cargo_dist_target() {
        let runner =
            FakeRunner::with_dist(Err(io::Error::new(io::ErrorKind::NotFound, "not found")));
        let plan = cargo_dist_plan();
        let mut state = state_with_completed(&[Phase::DryRun]);
        state
            .built
            .insert(journal_target_ids(&plan.targets).remove(0));
        check(&plan, Some(&state), &runner, Path::new("/repo"), None).unwrap();
        assert!(!runner
            .calls
            .borrow()
            .iter()
            .any(|call| call == "dist --version"));
    }

    #[test]
    fn resume_after_build_does_not_require_cargo_dist() {
        let runner =
            FakeRunner::with_dist(Err(io::Error::new(io::ErrorKind::NotFound, "not found")));
        let state = state_with_completed(&[Phase::DryRun, Phase::Build]);
        check(
            &cargo_dist_plan(),
            Some(&state),
            &runner,
            Path::new("/repo"),
            None,
        )
        .unwrap();
        assert!(!runner
            .calls
            .borrow()
            .iter()
            .any(|call| call == "dist --version"));
    }

    fn state_with_completed(phases: &[Phase]) -> RunState {
        let mut state = RunState::empty();
        state.run_id = "run".into();
        state.plan_id = "plan".into();
        state.version = "1.2.3".into();
        state.targets = vec!["target".into()];
        state.phases = phases
            .iter()
            .map(|phase| PhaseRecord {
                phase: *phase,
                outcome: PhaseOutcome::Ok,
            })
            .collect();
        state
    }
}
