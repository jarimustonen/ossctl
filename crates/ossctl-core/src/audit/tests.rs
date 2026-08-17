//! Unit tests for the readiness scorer — fixture repos at each maturity, the
//! GH-API-failure ⇒ `unknown` path, and a read-only assertion (no port mutation
//! exists to make).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::contract::schema::{
    Changelog, ChangelogMode, ChangelogSource, Contract, ContributionProvenance, DependencyBot,
    Distribution, DistributionAdapter, DocsSite, Ecosystem, HealthBadge, Maturity, ProvenanceLevel,
    Registry, Release, ReleaseLayout, ReleaseModel, Status, Target, VersioningBase,
};
use crate::ports::{CommandOutput, CommandRunner};
use crate::protocol::facts::{DistributionSurface, Facts, MaturitySignals};

// ── In-memory Fs fake (mirrors the facts module's) ─────────────────────────

#[derive(Default)]
struct FakeFs {
    files: HashMap<PathBuf, Vec<u8>>,
    dirs: HashSet<PathBuf>,
    /// Paths that a `read` fails on with `PermissionDenied` — used to exercise
    /// the "could not check ⇒ unknown" workflow-probe path.
    unreadable: HashSet<PathBuf>,
}

impl FakeFs {
    fn file(mut self, path: &str, contents: &str) -> Self {
        let p = PathBuf::from(path);
        let mut cur = p.parent();
        while let Some(dir) = cur {
            if dir.as_os_str().is_empty() {
                break;
            }
            self.dirs.insert(dir.to_path_buf());
            cur = dir.parent();
        }
        self.files.insert(p, contents.as_bytes().to_vec());
        self
    }

    fn dir(mut self, path: &str) -> Self {
        self.dirs.insert(PathBuf::from(path));
        self
    }

    /// Register a directory entry that exists (so `read_dir` lists it) but whose
    /// `read` fails with `PermissionDenied`.
    fn unreadable_file(mut self, path: &str) -> Self {
        let p = PathBuf::from(path);
        if let Some(dir) = p.parent() {
            self.dirs.insert(dir.to_path_buf());
        }
        self.files.insert(p.clone(), Vec::new());
        self.unreadable.insert(p);
        self
    }
}

impl Fs for FakeFs {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        if self.unreadable.contains(path) {
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        }
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }
    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path) || self.dirs.contains(path)
    }
    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.contains(path)
    }
    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }
    fn read_dir(&self, dir: &Path) -> std::io::Result<Vec<String>> {
        if !self.dirs.contains(dir) {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        }
        let mut names: Vec<String> = self
            .files
            .keys()
            .chain(self.dirs.iter())
            .filter(|p| p.parent() == Some(dir))
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        names.sort();
        Ok(names)
    }
}

// ── CommandRunner fake ─────────────────────────────────────────────────────

/// Programmable command runner: matches on `(program, args)` and records every
/// call so a test can assert exactly which read-only commands ran.
struct FakeCmd {
    responses: HashMap<String, CommandOutput>,
    calls: RefCell<Vec<String>>,
    /// When true, every unmatched call returns an `Err` (git/gh missing).
    err_on_miss: bool,
}

impl FakeCmd {
    fn new() -> Self {
        Self {
            responses: HashMap::new(),
            calls: RefCell::new(Vec::new()),
            err_on_miss: false,
        }
    }

    fn key(program: &str, args: &[&str]) -> String {
        format!("{program} {}", args.join(" "))
    }

    fn on(mut self, program: &str, args: &[&str], status: i32, stdout: &str, stderr: &str) -> Self {
        self.responses.insert(
            Self::key(program, args),
            CommandOutput {
                status: Some(status),
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            },
        );
        self
    }

    /// A runner with a working GitHub remote and a `community/profile` response.
    fn github(profile_json: &str) -> Self {
        Self::new()
            .on(
                "git",
                &["remote", "get-url", "origin"],
                0,
                "git@github.com:acme/tool.git\n",
                "",
            )
            .on(
                "gh",
                &["api", "repos/acme/tool/community/profile"],
                0,
                profile_json,
                "",
            )
    }
}

impl CommandRunner for FakeCmd {
    fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> std::io::Result<CommandOutput> {
        let key = Self::key(program, args);
        self.calls.borrow_mut().push(key.clone());
        match self.responses.get(&key) {
            Some(out) => Ok(out.clone()),
            None if self.err_on_miss => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
            None => Ok(CommandOutput {
                status: Some(1),
                stdout: String::new(),
                stderr: "not found".to_string(),
            }),
        }
    }
}

// ── Builders ───────────────────────────────────────────────────────────────

fn repo() -> &'static Path {
    Path::new("/repo")
}

/// A minimal, valid contract at the given maturity with a single crates.io
/// target and a real license — the common case a test tweaks from.
fn contract_at(maturity: Maturity) -> Contract {
    Contract {
        schema_version: 1,
        status: Status::Approved,
        maturity,
        ecosystems: vec![Ecosystem::Rust],
        targets: vec![Target {
            ecosystem: Ecosystem::Rust,
            package: Some("tool".to_string()),
            registry: Registry::CratesIo,
            adapter: crate::contract::schema::Adapter::CargoPublish,
        }],
        distributions: vec![],
        versioning: VersioningBase::Semver,
        versioning_pattern: None,
        changelog: Changelog {
            mode: ChangelogMode::Curated,
            source: ChangelogSource::Manual,
            fragment_dir: crate::contract::schema::DEFAULT_FRAGMENT_DIR.to_string(),
        },
        conventional_commits: false,
        release: Release {
            model: ReleaseModel::Gated,
            layout: ReleaseLayout::Single,
            bump_hook: None,
        },
        contribution_provenance: ContributionProvenance::None,
        provenance_level: ProvenanceLevel::None,
        dependency_bot: DependencyBot::None,
        health_badges: vec![],
        license: "MIT".to_string(),
        docs_site: DocsSite::None,
        extra_fields: serde_json::Map::new(),
        warnings: vec![],
    }
}

/// A binary-distribution block that builds exactly the given `platforms`.
fn dist_with(platforms: &[&str]) -> Distribution {
    Distribution {
        package: None,
        adapter: DistributionAdapter::CargoDist,
        gh_releases: true,
        installers: vec![],
        homebrew_tap: None,
        platforms: platforms.iter().map(|s| (*s).to_string()).collect(),
        extra_fields: serde_json::Map::new(),
    }
}

/// Facts with sensible defaults for a repo of the given CI/bot posture.
fn facts_with(maturity: Maturity, has_ci: bool, bot: Option<&str>) -> Facts {
    Facts {
        repo_root: "/repo".to_string(),
        is_git: true,
        has_commits: true,
        ecosystems: vec![Ecosystem::Rust],
        packages: vec![],
        committers_total: 1,
        committers_recent_year: 1,
        tags: vec![],
        has_semver_tag: false,
        has_ge_1_0_release: false,
        has_ci,
        dependency_bot: bot.map(str::to_string),
        has_issues_dir: false,
        readme_self_label: None,
        description: None,
        maturity_signals: MaturitySignals {
            production: false,
            spike: false,
        },
        inferred_maturity: maturity,
        distribution_surface: DistributionSurface {
            has_cargo_dist: false,
            cargo_dist_evidence: vec![],
            tag_triggered_workflows: vec![],
        },
        rust_workspace: None,
    }
}

fn ids(report: &AuditReport) -> Vec<&str> {
    report.gaps.iter().map(|g| g.id.as_str()).collect()
}

fn gap<'a>(report: &'a AuditReport, id: &str) -> &'a Gap {
    report
        .gaps
        .iter()
        .find(|g| g.id == id)
        .unwrap_or_else(|| panic!("expected a '{id}' gap, got {:?}", ids(report)))
}

/// A profile JSON where GitHub sees only a README + LICENSE.
const PROFILE_README_LICENSE: &str = r#"{"files":{
    "readme":{"url":"x"},"license":{"key":"mit"},
    "contributing":null,"code_of_conduct":null,
    "issue_template":null,"pull_request_template":null,"security":null}}"#;

// ── Gated core ─────────────────────────────────────────────────────────────

#[test]
fn empty_repo_at_mvp_fails_core_on_readme_license_ci() {
    let fs = FakeFs::default();
    let report = audit(
        repo(),
        &contract_at(Maturity::Mvp),
        &facts_with(Maturity::Mvp, false, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert_eq!(report.core_complete, CoreStatus::Incomplete);
    // README + LICENSE + CI are all blocking core gaps at mvp.
    for id in ["readme", "license", "ci"] {
        let g = gap(&report, id);
        assert_eq!(g.category, Category::Core, "{id} is core");
        assert_eq!(g.severity, Severity::Blocking, "{id} blocks at mvp");
    }
}

#[test]
fn spike_gates_on_readme_license_only_ci_is_recommended() {
    // A spike is not being published: CI is a canon gap toward mvp, NOT a core
    // failure. README + LICENSE present → core complete.
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract_at(Maturity::Spike),
        &facts_with(Maturity::Spike, false, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert_eq!(report.core_complete, CoreStatus::Complete);
    let ci = gap(&report, "ci");
    assert_eq!(ci.category, Category::Canon);
    assert_eq!(ci.severity, Severity::Recommended);
    // A spike does not get mvp canon gaps (changelog/contributing/…).
    assert!(!ids(&report).contains(&"changelog"));
    assert!(!ids(&report).contains(&"contributing"));
}

#[test]
fn complete_mvp_repo_has_no_core_gap() {
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file("/repo/CHANGELOG.md", "# Changelog\n")
        .file("/repo/CONTRIBUTING.md", "# Contributing\n")
        .file("/repo/CODE_OF_CONDUCT.md", "# CoC\n")
        .file("/repo/SECURITY.md", "# Security\n")
        .file("/repo/.github/workflows/ci.yml", "on: push\n");
    let report = audit(
        repo(),
        &contract_at(Maturity::Mvp),
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert_eq!(report.core_complete, CoreStatus::Complete);
    assert!(
        report.gaps.is_empty(),
        "expected no gaps, got {:?}",
        ids(&report)
    );
}

#[test]
fn license_found_in_github_subdir() {
    // GitHub recognizes health files under `.github/`; the probe must too.
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/.github/LICENSE.md", "MIT\n");
    let report = audit(
        repo(),
        &contract_at(Maturity::Spike),
        &facts_with(Maturity::Spike, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert!(!ids(&report).contains(&"license"));
    assert_eq!(report.core_complete, CoreStatus::Complete);
}

// ── Tier-scaled canon ──────────────────────────────────────────────────────

#[test]
fn mvp_reports_canon_gaps_when_absent() {
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file("/repo/.github/workflows/ci.yml", "on: push\n");
    let report = audit(
        repo(),
        &contract_at(Maturity::Mvp),
        &facts_with(Maturity::Mvp, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert_eq!(report.core_complete, CoreStatus::Complete);
    for id in [
        "changelog",
        "contributing",
        "code-of-conduct",
        "security-policy",
        "dependency-bot",
    ] {
        let g = gap(&report, id);
        assert_eq!(g.category, Category::Canon, "{id} is canon");
        assert_eq!(g.severity, Severity::Recommended, "{id} never blocks");
    }
    // production-only canon does not appear at mvp.
    assert!(!ids(&report).contains(&"codeowners"));
    assert!(!ids(&report).contains(&"architecture"));
}

#[test]
fn production_adds_hardening_canon_gaps() {
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file("/repo/CHANGELOG.md", "# Changelog\n")
        .file("/repo/CONTRIBUTING.md", "# c\n")
        .file("/repo/CODE_OF_CONDUCT.md", "# c\n")
        .file("/repo/SECURITY.md", "# s\n")
        .file("/repo/.github/workflows/ci.yml", "on: push\n");
    let report = audit(
        repo(),
        &contract_at(Maturity::Production),
        &facts_with(Maturity::Production, true, Some("renovate")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert_eq!(report.core_complete, CoreStatus::Complete);
    for id in [
        "codeowners",
        "governance",
        "architecture",
        "pre-commit",
        "coverage",
    ] {
        assert!(
            ids(&report).contains(&id),
            "expected {id} gap at production"
        );
    }
}

// ── Producer-existence gaps ────────────────────────────────────────────────

#[test]
fn fragment_changelog_without_dir_is_a_producer_gap() {
    let mut contract = contract_at(Maturity::Mvp);
    contract.changelog.mode = ChangelogMode::Fragment;
    contract.changelog.source = ChangelogSource::IssuectlTrailers;
    contract.changelog.fragment_dir = "changelog/fragments".to_string();
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file("/repo/.github/workflows/ci.yml", "on: push\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "changelog-fragment-dir");
    assert_eq!(g.category, Category::Producer);
    assert_eq!(g.severity, Severity::Recommended);
    assert!(g.detail.contains("changelog/fragments"));
}

#[test]
fn present_fragment_dir_yields_no_gap() {
    let mut contract = contract_at(Maturity::Mvp);
    contract.changelog.mode = ChangelogMode::Fragment;
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file("/repo/.github/workflows/ci.yml", "on: push\n")
        .dir("/repo/changelog/fragments");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert!(!ids(&report).contains(&"changelog-fragment-dir"));
}

#[test]
fn coverage_badge_without_coverage_step_is_producer_gap() {
    let mut contract = contract_at(Maturity::Mvp);
    contract.health_badges = vec![HealthBadge::Coverage];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file(
            "/repo/.github/workflows/ci.yml",
            "on: push\njobs:\n  test:\n",
        );
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "coverage");
    assert_eq!(g.category, Category::Producer);
    assert!(g.detail.contains("coverage"));
}

#[test]
fn coverage_badge_with_coverage_step_yields_no_gap() {
    let mut contract = contract_at(Maturity::Mvp);
    contract.health_badges = vec![HealthBadge::Coverage];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file(
            "/repo/.github/workflows/ci.yml",
            "jobs:\n  test:\n    steps:\n      - run: cargo llvm-cov\n",
        );
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert!(!ids(&report).contains(&"coverage"));
}

#[test]
fn scorecard_badge_without_action_is_producer_gap() {
    let mut contract = contract_at(Maturity::Mvp);
    contract.health_badges = vec![HealthBadge::Scorecard];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file("/repo/.github/workflows/ci.yml", "on: push\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "scorecard");
    assert_eq!(g.category, Category::Producer);
}

#[test]
fn registry_target_without_license_is_producer_gap() {
    let mut contract = contract_at(Maturity::Spike);
    contract.license = String::new(); // no SPDX license configured
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n"); // file exists, but contract declares none
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Spike, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "registry-license");
    assert_eq!(g.category, Category::Producer);
    assert_eq!(g.member, "oss-readme");
}

#[test]
fn publish_none_repo_is_audited_without_any_publish_gap() {
    // A publish-none contract (an authored `targets: []` — a private, never-published
    // repo) is a valid, honored state, not a half-configured one. The audit must not
    // demand publish infrastructure from it: no registry-license gap even with NO
    // license configured (nothing is being published to a registry that requires one),
    // and no cross-platform distribution gap (there is no distribution).
    let mut contract = contract_at(Maturity::Mvp);
    contract.targets = vec![];
    contract.license = String::new();
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file("/repo/CHANGELOG.md", "# Changelog\n")
        .file("/repo/CONTRIBUTING.md", "# Contributing\n")
        .file("/repo/CODE_OF_CONDUCT.md", "# CoC\n")
        .file("/repo/SECURITY.md", "# Security\n")
        .file("/repo/.github/workflows/ci.yml", "on: push\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert_eq!(report.core_complete, CoreStatus::Complete);
    assert!(
        report.gaps.is_empty(),
        "a complete publish-none repo has no gaps, got {:?}",
        ids(&report)
    );
}

// ── Cross-platform distribution policy (macOS AND Linux) ───────────────────

#[test]
fn distribution_without_linux_target_is_producer_gap() {
    // An explicit, macOS-only platform set builds no Linux binary → gap.
    let mut contract = contract_at(Maturity::Mvp);
    contract.distributions = vec![dist_with(&["aarch64-apple-darwin", "x86_64-apple-darwin"])];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "distribution-linux");
    assert_eq!(g.category, Category::Producer);
    assert_eq!(g.severity, Severity::Recommended);
    assert_eq!(g.member, "oss-init");
    assert!(g.detail.contains("Linux"));
    // macOS IS covered here, so only the Linux gap fires — the two OS checks are
    // independent.
    assert!(!ids(&report).contains(&"distribution-macos"));
}

#[test]
fn distribution_without_macos_target_is_producer_gap() {
    // Symmetric to the Linux case: a Linux-only set builds no macOS binary → the
    // macOS gap fires (and the Linux gap does not).
    let mut contract = contract_at(Maturity::Mvp);
    contract.distributions = vec![dist_with(&["x86_64-unknown-linux-gnu"])];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "distribution-macos");
    assert_eq!(g.category, Category::Producer);
    assert!(g.detail.contains("macOS"));
    assert!(!ids(&report).contains(&"distribution-linux"));
}

#[test]
fn distribution_missing_both_oses_yields_two_gaps() {
    // A Windows-only distribution covers neither macOS nor Linux → both gaps.
    let mut contract = contract_at(Maturity::Mvp);
    contract.distributions = vec![dist_with(&["x86_64-pc-windows-msvc"])];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert!(ids(&report).contains(&"distribution-linux"));
    assert!(ids(&report).contains(&"distribution-macos"));
}

#[test]
fn android_triple_does_not_satisfy_the_linux_requirement() {
    // `*-linux-android` is a Linux *kernel* target but not a desktop-Linux
    // install target: it must NOT satisfy the policy. (macOS is present here, so
    // only the Linux gap fires.)
    let mut contract = contract_at(Maturity::Mvp);
    contract.distributions = vec![dist_with(&[
        "aarch64-linux-android",
        "aarch64-apple-darwin",
    ])];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert!(ids(&report).contains(&"distribution-linux"));
    assert!(!ids(&report).contains(&"distribution-macos"));
}

#[test]
fn distribution_with_linux_target_yields_no_gap() {
    // The cross-platform default set (macOS + Linux musl) covers both → no gap.
    let mut contract = contract_at(Maturity::Mvp);
    contract.distributions = vec![dist_with(&[
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl",
    ])];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert!(!ids(&report).contains(&"distribution-linux"));
    assert!(!ids(&report).contains(&"distribution-macos"));
}

#[test]
fn platform_triple_classifiers() {
    // Table check for the OS classifiers behind the cross-platform gap.
    for t in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-musl"] {
        assert!(is_linux_triple(t), "{t} is Linux");
        assert!(!is_darwin_triple(t), "{t} is not macOS");
    }
    for t in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
        assert!(is_darwin_triple(t), "{t} is macOS");
        assert!(!is_linux_triple(t), "{t} is not Linux");
    }
    for t in [
        "x86_64-pc-windows-msvc",
        "aarch64-linux-android", // Linux kernel, not desktop-Linux
        "aarch64-apple-ios",     // Apple, not macOS
        "wasm32-unknown-unknown",
    ] {
        assert!(!is_linux_triple(t), "{t} is not desktop-Linux");
        assert!(!is_darwin_triple(t), "{t} is not macOS");
    }
}

#[test]
fn distribution_with_gnu_linux_target_yields_no_gap() {
    // A gnu (not musl) Linux triple + a macOS triple satisfies the policy.
    let mut contract = contract_at(Maturity::Mvp);
    contract.distributions = vec![dist_with(&[
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
    ])];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert!(!ids(&report).contains(&"distribution-linux"));
    assert!(!ids(&report).contains(&"distribution-macos"));
}

#[test]
fn no_distribution_block_yields_no_cross_platform_gap() {
    // A registry-only repo (no `distribution`) is never flagged for Linux.
    let contract = contract_at(Maturity::Production); // distribution: None
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Production, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert!(!ids(&report).contains(&"distribution-linux"));
    assert!(!ids(&report).contains(&"distribution-macos"));
}

#[test]
fn distribution_without_linux_escalates_wording_at_production() {
    // Same gap fires at production, but the detail marks it as policy-required.
    let mut contract = contract_at(Maturity::Production);
    contract.distributions = vec![dist_with(&["aarch64-apple-darwin"])];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Production, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "distribution-linux");
    assert!(g.detail.contains("required"), "detail: {}", g.detail);
}

#[test]
fn monorepo_distributions_yield_per_package_gap_ids() {
    // Two distributions, each missing a different OS: the gap ids are suffixed
    // with the package so they do not collide (bare ids are the single-dist case).
    let mut contract = contract_at(Maturity::Mvp);
    let mut alpha = dist_with(&["aarch64-apple-darwin"]); // no Linux
    alpha.package = Some("alpha".to_string());
    let mut beta = dist_with(&["x86_64-unknown-linux-musl"]); // no macOS
    beta.package = Some("beta".to_string());
    contract.distributions = vec![alpha, beta];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let all = ids(&report);
    assert!(all.contains(&"distribution-linux:alpha"), "ids: {all:?}");
    assert!(all.contains(&"distribution-macos:beta"), "ids: {all:?}");
    // The bare (single-dist) ids never appear when there are several.
    assert!(!all.contains(&"distribution-linux"));
    assert!(!all.contains(&"distribution-macos"));
}

// ── GitHub community standards + the unknown discipline ────────────────────

#[test]
fn community_profile_parsed_on_success() {
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract_at(Maturity::Spike),
        &facts_with(Maturity::Spike, true, None),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let cp = &report.community_profile;
    assert!(cp.checked);
    assert_eq!(cp.unavailable_reason, None);
    assert_eq!(cp.readme, Presence::Present);
    assert_eq!(cp.license, Presence::Present);
    // A file GitHub reports absent is Absent, not Unknown (it WAS checked).
    assert_eq!(cp.contributing, Presence::Absent);
    assert_eq!(cp.security, Presence::Absent);
}

#[test]
fn gh_api_failure_yields_unknown_never_false() {
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    // git remote resolves, but `gh api` fails (404 / offline).
    let cmd = FakeCmd::new()
        .on(
            "git",
            &["remote", "get-url", "origin"],
            0,
            "git@github.com:acme/tool.git\n",
            "",
        )
        .on(
            "gh",
            &["api", "repos/acme/tool/community/profile"],
            1,
            "",
            "gh: Not Found (HTTP 404)\n",
        );
    let report = audit(
        repo(),
        &contract_at(Maturity::Spike),
        &facts_with(Maturity::Spike, true, None),
        &fs,
        &cmd,
    );
    let cp = &report.community_profile;
    assert!(!cp.checked, "a failed lookup is not 'checked'");
    // EVERY field is unknown — never Absent — on an outage.
    for p in [
        cp.readme,
        cp.license,
        cp.contributing,
        cp.code_of_conduct,
        cp.issue_template,
        cp.pull_request_template,
        cp.security,
    ] {
        assert_eq!(
            p,
            Presence::Unknown,
            "outage must yield unknown, not absent"
        );
    }
    assert!(cp.unavailable_reason.as_deref().unwrap().contains("404"));
}

#[test]
fn non_github_remote_yields_unchecked_profile() {
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let cmd = FakeCmd::new().on(
        "git",
        &["remote", "get-url", "origin"],
        0,
        "git@gitlab.com:acme/tool.git\n",
        "",
    );
    let report = audit(
        repo(),
        &contract_at(Maturity::Spike),
        &facts_with(Maturity::Spike, true, None),
        &fs,
        &cmd,
    );
    assert!(!report.community_profile.checked);
    assert_eq!(report.community_profile.readme, Presence::Unknown);
    // No `gh api` call is attempted when there is no GitHub remote.
    assert!(!cmd.calls.borrow().iter().any(|c| c.starts_with("gh api")));
}

#[test]
fn no_remote_yields_unchecked_profile() {
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    // git remote get-url fails (exit 1: no origin).
    let cmd = FakeCmd::new().on("git", &["remote", "get-url", "origin"], 1, "", "error");
    let report = audit(
        repo(),
        &contract_at(Maturity::Spike),
        &facts_with(Maturity::Spike, true, None),
        &fs,
        &cmd,
    );
    assert!(!report.community_profile.checked);
}

// URL parsing (`parse_github_slug`) moved to `crate::vcs`; its tests live there.

// ── Producer probe: outage ⇒ unknown, never absent ─────────────────────────

#[test]
fn coverage_probe_read_failure_yields_unknown_not_absent() {
    // The workflows dir lists a file, but reading it fails (permission). The
    // coverage badge's producer cannot be confirmed absent ⇒ Unknown gap.
    let mut contract = contract_at(Maturity::Mvp);
    contract.health_badges = vec![HealthBadge::Coverage];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .unreadable_file("/repo/.github/workflows/ci.yml");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "coverage");
    assert_eq!(
        g.status,
        Presence::Unknown,
        "an unreadable workflow must yield unknown, not absent"
    );
}

#[test]
fn coverage_probe_ignores_non_yaml_files() {
    // A non-YAML file mentioning 'coverage' must NOT satisfy the producer.
    let mut contract = contract_at(Maturity::Mvp);
    contract.health_badges = vec![HealthBadge::Coverage];
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n")
        .file("/repo/.github/workflows/notes.txt", "coverage is planned\n");
    let report = audit(
        repo(),
        &contract,
        &facts_with(Maturity::Mvp, true, Some("dependabot")),
        &fs,
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let g = gap(&report, "coverage");
    assert_eq!(g.status, Presence::Absent, "a .txt file is not a workflow");
}

// ── GitHub community profile: schema robustness + security alias ────────────

#[test]
fn community_profile_missing_files_object_is_unknown_not_absent() {
    // A 200 with a valid-JSON but unexpected body (no `files` object) must NOT
    // report every file absent — it is "could not check".
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let cmd = FakeCmd::new()
        .on(
            "git",
            &["remote", "get-url", "origin"],
            0,
            "git@github.com:acme/tool.git\n",
            "",
        )
        .on(
            "gh",
            &["api", "repos/acme/tool/community/profile"],
            0,
            r#"{"message":"rate limited"}"#,
            "",
        );
    let report = audit(
        repo(),
        &contract_at(Maturity::Spike),
        &facts_with(Maturity::Spike, true, None),
        &fs,
        &cmd,
    );
    let cp = &report.community_profile;
    assert!(!cp.checked, "no files object ⇒ not a successful check");
    assert_eq!(cp.readme, Presence::Unknown);
    assert_eq!(cp.security, Presence::Unknown);
}

#[test]
fn community_profile_reads_security_policy_alias() {
    // GitHub names the field `security_policy`; a present SECURITY.md there must
    // be reported present, not absent.
    let profile = r#"{"files":{
        "readme":{"url":"x"},"license":{"key":"mit"},
        "security_policy":{"url":"y"}}}"#;
    let fs = FakeFs::default()
        .file("/repo/README.md", "# tool\n")
        .file("/repo/LICENSE", "MIT\n");
    let report = audit(
        repo(),
        &contract_at(Maturity::Spike),
        &facts_with(Maturity::Spike, true, None),
        &fs,
        &FakeCmd::github(profile),
    );
    assert_eq!(report.community_profile.security, Presence::Present);
}

// ── Wire-format contract (serialized enum strings) ─────────────────────────

#[test]
fn enum_wire_strings_are_stable() {
    // The JSON strings downstream consumers key off must not drift; assert the
    // exact serialization for every enum variant.
    let cases = [
        (
            serde_json::to_string(&Presence::Present).unwrap(),
            "\"present\"",
        ),
        (
            serde_json::to_string(&Presence::Absent).unwrap(),
            "\"absent\"",
        ),
        (
            serde_json::to_string(&Presence::Unknown).unwrap(),
            "\"unknown\"",
        ),
        (
            serde_json::to_string(&CoreStatus::Complete).unwrap(),
            "\"complete\"",
        ),
        (
            serde_json::to_string(&CoreStatus::Incomplete).unwrap(),
            "\"incomplete\"",
        ),
        (serde_json::to_string(&Category::Core).unwrap(), "\"core\""),
        (
            serde_json::to_string(&Category::Canon).unwrap(),
            "\"canon\"",
        ),
        (
            serde_json::to_string(&Category::Producer).unwrap(),
            "\"producer\"",
        ),
        (
            serde_json::to_string(&Severity::Blocking).unwrap(),
            "\"blocking\"",
        ),
        (
            serde_json::to_string(&Severity::Recommended).unwrap(),
            "\"recommended\"",
        ),
    ];
    for (got, want) in cases {
        assert_eq!(got, want, "wire string drift");
    }
    // as_str() and the Serialize derive agree (unquoted).
    assert_eq!(Presence::Unknown.as_str(), "unknown");
    assert_eq!(CoreStatus::Incomplete.as_str(), "incomplete");
    assert_eq!(Category::Producer.as_str(), "producer");
    assert_eq!(Severity::Blocking.as_str(), "blocking");
}

// ── Determinism ────────────────────────────────────────────────────────────

#[test]
fn same_inputs_same_report() {
    let build_fs = || {
        FakeFs::default()
            .file("/repo/README.md", "# tool\n")
            .file("/repo/LICENSE", "MIT\n")
            .file("/repo/.github/workflows/ci.yml", "on: push\n")
    };
    let a = audit(
        repo(),
        &contract_at(Maturity::Mvp),
        &facts_with(Maturity::Mvp, true, None),
        &build_fs(),
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    let b = audit(
        repo(),
        &contract_at(Maturity::Mvp),
        &facts_with(Maturity::Mvp, true, None),
        &build_fs(),
        &FakeCmd::github(PROFILE_README_LICENSE),
    );
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}
