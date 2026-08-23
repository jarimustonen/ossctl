//! `bump_exec` tests: real temp checkout (so the `std::fs` manifest/CHANGELOG edits are
//! exercised for real) + a recording fake [`CommandRunner`] (so `cargo`/`git`/the hook
//! are deterministic and no real toolchain is needed). Each asserts the edited file
//! contents and the fail-closed behaviour on a bad edit / hook.

use std::cell::RefCell;
use std::io;
use std::path::Path;

use tempfile::TempDir;

use super::*;
use crate::contract::schema::{ChangelogMode, ChangelogSource};
use crate::ports::{Clock, CommandOutput, CommandRunner, RegistryQuery};
use crate::protocol::plan::{BumpLevel, BumpPlan, ChangelogFinalizePlan, PinRewrite};
use crate::release::adapters::{EffectCtx, EMPTY_ARTIFACTS};

/// A fake runner that records calls and returns success by default. `git rev-parse
/// HEAD` yields a fixed bump-commit sha; a `fail_substr` makes any matching command
/// fail; a `hook_fail` makes `sh -c` fail. File I/O is **not** faked — the executor
/// writes into a real temp dir the tests set up.
struct FakeRunner {
    calls: RefCell<Vec<String>>,
    bump_commit: String,
    fail_substr: Option<String>,
    hook_fail: bool,
    issuectl_json: Option<String>,
    /// A callback run when `sh -c <hook>` fires, so a test can model a hook that edits
    /// files in the checkout (e.g. regenerating a snapshot, or maliciously re-versioning).
    hook_effect: Option<HookEffect>,
}

/// A test hook side-effect: given the checkout root, mutate files as a real hook would.
type HookEffect = Box<dyn Fn(&Path)>;

impl FakeRunner {
    fn new(bump_commit: &str) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            bump_commit: bump_commit.to_string(),
            fail_substr: None,
            hook_fail: false,
            issuectl_json: None,
            hook_effect: None,
        }
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str], cwd: &Path) -> io::Result<CommandOutput> {
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        if let Some(sub) = &self.fail_substr {
            if line.contains(sub.as_str()) {
                return Ok(CommandOutput {
                    status: Some(1),
                    stdout: String::new(),
                    stderr: "boom".into(),
                });
            }
        }
        if program == "sh" {
            if let Some(effect) = &self.hook_effect {
                effect(cwd);
            }
            return Ok(CommandOutput {
                status: Some(i32::from(self.hook_fail)),
                stdout: String::new(),
                stderr: if self.hook_fail {
                    "hook boom".into()
                } else {
                    String::new()
                },
            });
        }
        if program == "issuectl" {
            return Ok(CommandOutput {
                status: Some(i32::from(self.issuectl_json.is_none())),
                stdout: self.issuectl_json.clone().unwrap_or_default(),
                stderr: String::new(),
            });
        }
        if program == "git" && args == ["rev-parse", "HEAD"] {
            return Ok(CommandOutput {
                status: Some(0),
                stdout: format!("{}\n", self.bump_commit),
                stderr: String::new(),
            });
        }
        Ok(CommandOutput {
            status: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

struct FakeClock;
impl Clock for FakeClock {
    fn now_unix(&self) -> u64 {
        0
    }
}
struct NoRegistry;
impl RegistryQuery for NoRegistry {
    fn published_versions(&self, _e: &str, _p: &str) -> io::Result<Vec<String>> {
        Ok(vec![])
    }
}

/// A unique throwaway workspace (root manifest + a lib + a bin pinning the lib + CHANGELOG).
/// The directory is removed when its [`TempDir`] owner drops.
fn temp_workspace() -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("shipshape-bump-test-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("crates/core")).unwrap();
    std::fs::create_dir_all(root.join("crates/cli")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/core\", \"crates/cli\"]\n\n[workspace.package]\nversion = \"0.4.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/core/Cargo.toml"),
        "[package]\nname = \"acme-core\"\nversion.workspace = true\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/cli/Cargo.toml"),
        "[package]\nname = \"acme\"\nversion.workspace = true\n\n[dependencies]\nacme-core = { path = \"../core\", version = \"=0.4.0\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("CHANGELOG.md"),
        "# Changelog\n\n## [Unreleased]\n### Added\n- a feature\n",
    )
    .unwrap();
    // A tracked lockfile, so the lockfile-refresh step runs (it is skipped when absent).
    std::fs::write(
        root.join("Cargo.lock"),
        "# auto\n[[package]]\nname = \"acme-core\"\nversion = \"0.4.0\"\n",
    )
    .unwrap();
    dir
}

/// A single-crate root manifest, with the same tracked lockfile and CHANGELOG surfaces
/// as [`temp_workspace`].
fn temp_single_crate() -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("shipshape-single-crate-bump-test-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"acme\"\nversion = \"0.4.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("CHANGELOG.md"),
        "# Changelog\n\n## [Unreleased]\n### Added\n- a feature\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Cargo.lock"),
        "# auto\n[[package]]\nname = \"acme\"\nversion = \"0.4.0\"\n",
    )
    .unwrap();
    dir
}

fn bump_plan() -> BumpPlan {
    BumpPlan {
        level: BumpLevel::Minor,
        from_version: "0.4.0".into(),
        to_version: "0.5.0".into(),
        pin_rewrites: vec![PinRewrite {
            in_package: "acme".into(),
            workspace_root: false,
            dependency: "acme-core".into(),
            from: "=0.4.0".into(),
            to: "=0.5.0".into(),
        }],
        changelog_finalize: true,
        changelog: None,
        bump_hook: None,
    }
}

fn ctx<'a>(
    runner: &'a FakeRunner,
    clock: &'a FakeClock,
    reg: &'a NoRegistry,
    root: &'a Path,
) -> EffectCtx<'a> {
    EffectCtx {
        runner,
        clock,
        registry: reg,
        repo_root: root,
        artifacts: &EMPTY_ARTIFACTS,
    }
}

#[test]
fn applies_version_pin_changelog_and_commits() {
    let dir = temp_workspace();
    let runner = FakeRunner::new("abc123def456");
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());

    let outcome = apply_bump(&ctx, &bump_plan(), "2026-08-13").unwrap();
    assert_eq!(outcome.commit, "abc123def456");
    assert_eq!(outcome.effective_date, "2026-08-13");

    let root = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    assert!(
        root.contains("version = \"0.5.0\""),
        "workspace version bumped: {root}"
    );
    let cli = std::fs::read_to_string(dir.path().join("crates/cli/Cargo.toml")).unwrap();
    assert!(cli.contains("version = \"=0.5.0\""), "pin rewritten: {cli}");
    let changelog = std::fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(
        changelog.contains("## [Unreleased]\n\n## [0.5.0] - 2026-08-13"),
        "changelog finalized: {changelog}"
    );

    // The lockfile refresh + commit went through the runner.
    let calls = runner.calls.borrow();
    assert!(calls.iter().any(|c| c == "cargo update --workspace"));
    assert!(calls
        .iter()
        .any(|c| c.starts_with("git commit -m release: v0.5.0")));
    // The executor does NOT advance the branch (no pre-publish push).
    assert!(
        !calls.iter().any(|c| c.starts_with("git push")),
        "the executor must not push the branch: {calls:?}"
    );
}

#[test]
fn fragment_finalize_reproduces_and_repairs_the_project_canon_cut_failure() {
    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("CHANGELOG.md"),
        "# Changelog\n\n<!-- oss-changelog:unreleased-start -->\n## [Unreleased]\n\n### Added\n\n### Changed\n\n### Fixed\n<!-- oss-changelog:unreleased-end -->\n\n## [0.4.0] - 2026-08-01\n\nOld notes.\n",
    )
    .unwrap();
    let fragments = dir.path().join("changelog/fragments");
    std::fs::create_dir_all(&fragments).unwrap();
    std::fs::write(fragments.join("README.md"), "Fragment instructions.\n").unwrap();
    let fragment = fragments.join("agent-skills-terminology.md");
    std::fs::write(
        &fragment,
        "### Changed\n\n- Describe skills using Agent Skills terminology.\n",
    )
    .unwrap();

    let mut runner = FakeRunner::new("projectcanon062");
    runner.issuectl_json = Some(
        r#"{"schema_version":1,"data":{"groups":{"improvement":[{"slug":"skill-description-length-check","title":"Enforce the skill description limit"}]}}}"#.into(),
    );
    let (clock, reg) = (FakeClock, NoRegistry);
    let mut plan = bump_plan();
    plan.changelog = Some(ChangelogFinalizePlan {
        mode: ChangelogMode::Fragment,
        source: ChangelogSource::IssuectlTrailers,
        fragment_dir: "changelog/fragments".into(),
        issuectl_range: Some("v0.4.0..HEAD".into()),
    });

    apply_bump(&ctx(&runner, &clock, &reg, dir.path()), &plan, "2026-08-23").unwrap();

    let changelog = std::fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    let marker_end = changelog
        .find("<!-- oss-changelog:unreleased-end -->")
        .unwrap();
    let release = changelog.find("## [0.5.0] - 2026-08-23").unwrap();
    assert!(
        release > marker_end,
        "release escaped marker block: {changelog}"
    );
    assert!(changelog.contains("Describe skills using Agent Skills terminology"));
    assert!(changelog.contains("Enforce the skill description limit"));
    assert_eq!(
        changelog.matches("### Changed").count(),
        2,
        "skeleton + one merged release section: {changelog}"
    );
    assert_eq!(
        changelog
            .matches("<!-- oss-changelog:unreleased-end -->")
            .count(),
        1
    );
    assert!(!fragment.exists(), "compiled fragment must be consumed");
    assert!(
        fragments.join("README.md").exists(),
        "README is not a fragment"
    );
    assert!(runner
        .calls
        .borrow()
        .iter()
        .any(|call| { call.starts_with("issuectl changelog v0.4.0..HEAD --json --root") }));
}

#[test]
fn invalid_issuectl_output_falls_back_to_fragments() {
    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("CHANGELOG.md"),
        "<!-- oss-changelog:unreleased-start -->\n## [Unreleased]\n### Added\n### Changed\n### Fixed\n<!-- oss-changelog:unreleased-end -->\n",
    )
    .unwrap();
    let fragments = dir.path().join("changelog/fragments");
    std::fs::create_dir_all(&fragments).unwrap();
    std::fs::write(
        fragments.join("fallback.md"),
        "### Fixed\n\n- Fragment fallback.\n",
    )
    .unwrap();
    let mut runner = FakeRunner::new("fallback123");
    runner.issuectl_json = Some("not-json".into());
    let (clock, reg) = (FakeClock, NoRegistry);
    let mut plan = bump_plan();
    plan.changelog = Some(ChangelogFinalizePlan {
        mode: ChangelogMode::Fragment,
        source: ChangelogSource::IssuectlTrailers,
        fragment_dir: "changelog/fragments".into(),
        issuectl_range: Some("v0.4.0..HEAD".into()),
    });

    apply_bump(&ctx(&runner, &clock, &reg, dir.path()), &plan, "2026-08-23").unwrap();
    let changelog = std::fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("- Fragment fallback."));
    assert!(!fragments.join("fallback.md").exists());
}

#[test]
fn missing_changelog_fails_and_preserves_fragments() {
    let dir = temp_workspace();
    std::fs::remove_file(dir.path().join("CHANGELOG.md")).unwrap();
    let fragments = dir.path().join("changelog/fragments");
    std::fs::create_dir_all(&fragments).unwrap();
    let fragment = fragments.join("pending.md");
    std::fs::write(&fragment, "### Fixed\n\n- Pending.\n").unwrap();
    let runner = FakeRunner::new("unused");
    let (clock, reg) = (FakeClock, NoRegistry);
    let mut plan = bump_plan();
    plan.changelog = Some(ChangelogFinalizePlan {
        mode: ChangelogMode::Fragment,
        source: ChangelogSource::Manual,
        fragment_dir: "changelog/fragments".into(),
        issuectl_range: None,
    });

    let err = apply_bump(&ctx(&runner, &clock, &reg, dir.path()), &plan, "2026-08-23").unwrap_err();
    assert!(matches!(
        err,
        BumpExecError::Edit(BumpEditError::ChangelogUnreleasedNotFound)
    ));
    assert!(fragment.exists());
}

#[cfg(unix)]
#[test]
fn fragment_directory_cannot_escape_through_an_intermediate_symlink() {
    use std::os::unix::fs::symlink;

    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("CHANGELOG.md"),
        "<!-- oss-changelog:unreleased-start -->\n## [Unreleased]\n### Added\n### Changed\n### Fixed\n<!-- oss-changelog:unreleased-end -->\n",
    )
    .unwrap();
    let external = tempfile::tempdir().unwrap();
    let external_fragments = external.path().join("fragments");
    std::fs::create_dir_all(&external_fragments).unwrap();
    let outside = external_fragments.join("outside.md");
    std::fs::write(&outside, "### Fixed\n\n- Outside.\n").unwrap();
    symlink(external.path(), dir.path().join("changelog")).unwrap();
    let runner = FakeRunner::new("unused");
    let (clock, reg) = (FakeClock, NoRegistry);
    let mut plan = bump_plan();
    plan.changelog = Some(ChangelogFinalizePlan {
        mode: ChangelogMode::Fragment,
        source: ChangelogSource::Manual,
        fragment_dir: "changelog/fragments".into(),
        issuectl_range: None,
    });

    let err = apply_bump(&ctx(&runner, &clock, &reg, dir.path()), &plan, "2026-08-23").unwrap_err();
    assert!(matches!(err, BumpExecError::ChangelogCompile(_)));
    assert!(outside.exists(), "outside fragment must never be consumed");
}

#[test]
fn conflicting_finalize_keeps_fragment_for_a_safe_retry() {
    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("CHANGELOG.md"),
        "<!-- oss-changelog:unreleased-start -->\n## [Unreleased]\n### Added\n### Changed\n### Fixed\n<!-- oss-changelog:unreleased-end -->\n\n## [0.5.0] - 2026-08-22\n\nExisting.\n",
    )
    .unwrap();
    let fragments = dir.path().join("changelog/fragments");
    std::fs::create_dir_all(&fragments).unwrap();
    let fragment = fragments.join("pending.md");
    std::fs::write(&fragment, "### Fixed\n\n- Pending fix.\n").unwrap();
    let runner = FakeRunner::new("unused");
    let (clock, reg) = (FakeClock, NoRegistry);
    let mut plan = bump_plan();
    plan.changelog = Some(ChangelogFinalizePlan {
        mode: ChangelogMode::Fragment,
        source: ChangelogSource::Manual,
        fragment_dir: "changelog/fragments".into(),
        issuectl_range: None,
    });

    let err = apply_bump(&ctx(&runner, &clock, &reg, dir.path()), &plan, "2026-08-23").unwrap_err();
    assert!(matches!(
        err,
        BumpExecError::Edit(BumpEditError::ChangelogReleaseConflict { .. })
    ));
    assert!(
        fragment.exists(),
        "a failed finalize must not consume fragments"
    );
    assert!(!runner
        .calls
        .borrow()
        .iter()
        .any(|call| call.starts_with("git commit")));
}

#[test]
fn applies_inherited_workspace_pin_before_local_path_checks() {
    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/core\", \"crates/cli\"]\n\n[workspace.package]\nversion = \"0.4.0\"\nedition = \"2021\"\n\n[workspace.dependencies]\nacme-core = {\n  path = \"crates/core\",\n  version = \"=0.4.0\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("crates/cli/Cargo.toml"),
        "[package]\nname = \"acme\"\nversion.workspace = true\n\n[dependencies]\nacme-core.workspace = true\n",
    )
    .unwrap();
    let mut plan = bump_plan();
    plan.pin_rewrites = vec![PinRewrite {
        in_package: "workspace".into(),
        workspace_root: true,
        dependency: "acme-core".into(),
        from: "=0.4.0".into(),
        to: "=0.5.0".into(),
    }];
    let runner = FakeRunner::new("abc123def456");
    let (clock, reg) = (FakeClock, NoRegistry);

    apply_bump(&ctx(&runner, &clock, &reg, dir.path()), &plan, "2026-08-13").unwrap();

    let root = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    assert!(
        root.contains("[workspace.package]\nversion = \"0.5.0\""),
        "workspace version retained across the root pin rewrite: {root}"
    );
    assert!(
        root.contains("version = \"=0.5.0\""),
        "root pin rewritten: {root}"
    );
    let cli = std::fs::read_to_string(dir.path().join("crates/cli/Cargo.toml")).unwrap();
    assert!(cli.contains("acme-core.workspace = true"));
}

#[test]
fn applies_one_sealed_rewrite_to_normal_and_dev_dependency_pins() {
    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("crates/cli/Cargo.toml"),
        "[package]\nname = \"acme\"\nversion.workspace = true\n\n[dependencies]\nacme-core = { path = \"../core\", version = \"=0.4.0\" }\n\n[dev-dependencies]\nacme-core = { path = \"../core\", version = \"=0.4.0\" }\n",
    )
    .unwrap();
    let runner = FakeRunner::new("duplicate123");
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());

    apply_bump(&ctx, &bump_plan(), "2026-08-13").unwrap();

    let cli = std::fs::read_to_string(dir.path().join("crates/cli/Cargo.toml")).unwrap();
    assert_eq!(cli.matches("version = \"=0.5.0\"").count(), 2, "{cli}");
    assert!(!cli.contains("version = \"=0.4.0\""));
}

#[test]
fn applies_version_lockfile_changelog_and_commits_for_a_single_crate() {
    let dir = temp_single_crate();
    let runner = FakeRunner::new("single123");
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());
    let mut plan = bump_plan();
    plan.pin_rewrites.clear();

    let outcome = apply_bump(&ctx, &plan, "2026-08-13").unwrap();
    assert_eq!(outcome.commit, "single123");

    let manifest = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[package]\nname = \"acme\"\nversion = \"0.5.0\""));
    let changelog = std::fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("## [Unreleased]\n\n## [0.5.0] - 2026-08-13"));
    let calls = runner.calls.borrow();
    assert!(calls.iter().any(|c| c == "cargo update --workspace"));
    assert!(calls
        .iter()
        .any(|c| c.starts_with("git commit -m release: v0.5.0")));
}

#[test]
fn fails_closed_when_single_crate_version_does_not_match_the_sealed_plan() {
    let dir = temp_single_crate();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"acme\"\nversion = \"0.5.0\"\n",
    )
    .unwrap();
    let runner = FakeRunner::new("abc");
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());
    let mut plan = bump_plan();
    plan.pin_rewrites.clear();

    let err = apply_bump(&ctx, &plan, "2026-08-13").unwrap_err();
    assert!(matches!(
        err,
        BumpExecError::Edit(BumpEditError::RootManifestVersionNotFound)
    ));
    assert!(runner.calls.borrow().is_empty());
}

#[test]
fn skips_the_lockfile_refresh_when_no_lockfile_is_tracked() {
    let dir = temp_workspace();
    std::fs::remove_file(dir.path().join("Cargo.lock")).unwrap();
    let runner = FakeRunner::new("abc");
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());
    apply_bump(&ctx, &bump_plan(), "2026-08-13").unwrap();
    assert!(
        !runner
            .calls
            .borrow()
            .iter()
            .any(|c| c == "cargo update --workspace"),
        "no lockfile ⇒ no refresh: {:?}",
        runner.calls.borrow()
    );
}

#[test]
fn fails_closed_on_a_missing_pin() {
    let dir = temp_workspace();
    // The cli manifest pins `^0.4`, not `=0.4.0` — the sealed pin will not match.
    std::fs::write(
        dir.path().join("crates/cli/Cargo.toml"),
        "[package]\nname = \"acme\"\n\n[dependencies]\nacme-core = { path = \"../core\", version = \"^0.4\" }\n",
    )
    .unwrap();
    let runner = FakeRunner::new("abc");
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());

    let err = apply_bump(&ctx, &bump_plan(), "2026-08-13").unwrap_err();
    assert!(matches!(
        err,
        BumpExecError::Edit(BumpEditError::PinNotFound { .. })
    ));
    // No commit happened.
    assert!(!runner
        .calls
        .borrow()
        .iter()
        .any(|c| c.starts_with("git commit")));
}

#[test]
fn fails_closed_when_neither_root_version_shape_is_present() {
    let dir = temp_single_crate();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"acme\"\n",
    )
    .unwrap();
    let runner = FakeRunner::new("abc");
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());
    let mut plan = bump_plan();
    plan.pin_rewrites.clear();

    let err = apply_bump(&ctx, &plan, "2026-08-13").unwrap_err();
    assert!(matches!(
        err,
        BumpExecError::Edit(BumpEditError::RootManifestVersionNotFound)
    ));
    assert!(!runner
        .calls
        .borrow()
        .iter()
        .any(|call| call.starts_with("cargo update") || call.starts_with("git commit")));
}

#[test]
fn fails_closed_when_the_hook_fails() {
    let dir = temp_workspace();
    let mut runner = FakeRunner::new("abc");
    runner.hook_fail = true;
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());
    let mut plan = bump_plan();
    plan.bump_hook = Some("cargo insta test --accept".into());

    let err = apply_bump(&ctx, &plan, "2026-08-13").unwrap_err();
    assert!(matches!(err, BumpExecError::Hook { .. }));
}

#[test]
fn fails_closed_when_a_hook_reverts_a_single_crate_version() {
    let dir = temp_single_crate();
    let mut runner = FakeRunner::new("abc");
    runner.hook_effect = Some(Box::new(|cwd: &Path| {
        std::fs::write(
            cwd.join("Cargo.toml"),
            "[package]\nname = \"acme\"\nversion = \"9.9.9\"\n",
        )
        .unwrap();
    }));
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());
    let mut plan = bump_plan();
    plan.pin_rewrites.clear();
    plan.bump_hook = Some("evil".into());

    let err = apply_bump(&ctx, &plan, "2026-08-13").unwrap_err();
    assert!(matches!(err, BumpExecError::HookViolatedVersion { .. }));
}

#[test]
fn fails_closed_when_the_hook_reverts_the_version() {
    let dir = temp_workspace();
    let mut runner = FakeRunner::new("abc");
    // The hook rewrites the workspace version back to something else.
    runner.hook_effect = Some(Box::new(|cwd: &Path| {
        std::fs::write(
            cwd.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/core\", \"crates/cli\"]\n\n[workspace.package]\nversion = \"9.9.9\"\n",
        )
        .unwrap();
    }));
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());
    let mut plan = bump_plan();
    plan.pin_rewrites.clear();
    plan.bump_hook = Some("evil".into());

    let err = apply_bump(&ctx, &plan, "2026-08-13").unwrap_err();
    assert!(
        matches!(err, BumpExecError::HookViolatedVersion { .. }),
        "expected a hook-version violation, got {err}"
    );
}

#[test]
fn civil_date_converts_known_timestamps() {
    // 2026-08-13T00:00:00Z = 1_786_579_200 (a fixed reference).
    assert_eq!(civil_date(1_786_579_200), "2026-08-13");
    // The Unix epoch.
    assert_eq!(civil_date(0), "1970-01-01");
    // A leap day: 2024-02-29T12:00:00Z.
    assert_eq!(civil_date(1_709_208_000), "2024-02-29");
}

#[test]
fn fails_closed_when_lockfile_refresh_fails() {
    let dir = temp_workspace();
    let mut runner = FakeRunner::new("abc");
    runner.fail_substr = Some("cargo update".into());
    let (clock, reg) = (FakeClock, NoRegistry);
    let ctx = ctx(&runner, &clock, &reg, dir.path());

    let err = apply_bump(&ctx, &bump_plan(), "2026-08-13").unwrap_err();
    assert!(matches!(err, BumpExecError::LockRefresh(_)));
}
