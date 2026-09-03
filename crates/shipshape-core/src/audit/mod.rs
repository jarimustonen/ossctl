//! Readiness scoring over the normalized contract + detected facts (ADR-0001 §3).
//!
//! [`audit`] is a read-only function of `(repo tree, contract, facts)` that
//! produces a gap-report: the **gated core** (README + LICENSE + CI, tier-scaled
//! so a `spike` is gated on README + LICENSE alone), the **tier-scaled canon**
//! (recommended artifacts scaled to the contract's maturity), the
//! **producer-existence** obligations the contract declared (a `fragment`
//! changelog needs its dir, a `coverage`/`scorecard` badge needs its CI
//! producer, a registry target needs an SPDX license), and the **GitHub
//! community standards** (`gh api …/community/profile`). Feeds `shipshape audit`
//! and the `/shipshape-readiness` skill.
//!
//! **Read-only, always.** Every probe goes through the [`Fs`] and
//! [`CommandRunner`] ports; nothing here writes the repo. The `git remote` and
//! `gh api` calls are read-only. A registry/GitHub lookup that *fails* yields
//! [`Presence::Unknown`], never [`Presence::Absent`] — an outage is never read
//! as "the artifact is missing" (issue: registry/GH-API failure ⇒ `unknown`,
//! never `false`).
//!
//! The engine takes the already-normalized [`Contract`] and detected [`Facts`]
//! by reference — it never re-parses `OSS-RELEASE.md` nor re-derives facts. The
//! `shipshape-cli` handler runs `contract::normalize` and `facts::gather` (the same
//! code paths behind `contract show` and `facts`) and hands their results here,
//! so the audit, `/shipshape-init`, and every other member agree on maturity and the
//! gated core down to the byte (ADR-0001 §3).

use std::path::Path;

use crate::contract::schema::{ChangelogMode, Contract, HealthBadge, Maturity, Registry};
use crate::ports::{CommandRunner, Fs};
use crate::protocol::audit::{
    AuditReport, Category, CommunityProfile, CoreStatus, Gap, Presence, Severity,
};
use crate::protocol::facts::Facts;

mod publicize;

/// Score the repo at `repo_root` against its `contract` and detected `facts`.
///
/// Pure over its inputs and read-only over the repo: filesystem probes go
/// through `fs`; the GitHub community-standards lookup goes through `cmd`
/// (`git remote get-url origin` then `gh api …/community/profile`). Never
/// mutates anything.
#[must_use]
pub fn audit(
    repo_root: &Path,
    contract: &Contract,
    facts: &Facts,
    fs: &dyn Fs,
    cmd: &dyn CommandRunner,
) -> AuditReport {
    let maturity = contract.maturity;
    let mut gaps: Vec<Gap> = Vec::new();

    // ── Gated core (README + LICENSE always; CI at mvp+) ──
    let readme_present = probe(fs, repo_root, README_NAMES);
    let license_present = probe(fs, repo_root, LICENSE_NAMES);
    let ci_present = facts.has_ci;

    let mut core_incomplete = false;
    if !readme_present {
        core_incomplete = true;
        gaps.push(core_gap(
            "readme",
            "shipshape-readme",
            "no README found — the project's front door is part of the gated core",
        ));
    }
    if !license_present {
        core_incomplete = true;
        gaps.push(core_gap(
            "license",
            "shipshape-readme",
            "no LICENSE file found — a public release needs an SPDX-identified license \
             (part of the gated core)",
        ));
    }
    // CI only gates the core at mvp+; a spike gets CI reported as a canon gap
    // toward mvp, not a core failure (design §4).
    let ci_gates_core = tier_rank(maturity) >= tier_rank(Maturity::Mvp);
    if !ci_present {
        if ci_gates_core {
            core_incomplete = true;
            gaps.push(core_gap(
                "ci",
                "shipshape-ci",
                "no CI configuration found — test+lint on every PR is part of the gated \
                 core at mvp and above",
            ));
        } else {
            gaps.push(canon_gap(
                "ci",
                "shipshape-ci",
                Presence::Absent,
                "no CI configuration found — add test+lint on PR to reach mvp/publish",
            ));
        }
    }
    let core_complete = if core_incomplete {
        CoreStatus::Incomplete
    } else {
        CoreStatus::Complete
    };

    // ── Tier-scaled canon (recommended; never blocking) ──
    canon_gaps(&mut gaps, repo_root, facts, fs, maturity);

    // ── Producer-existence obligations declared by the contract ──
    // The normalizer does NOT hard-fail on a missing producer (advisory-producer
    // decision from the shipshape-init unit); the audit reports them as gaps.
    producer_gaps(&mut gaps, repo_root, contract, facts, fs, maturity);

    // ── Public-facing consistency checks (read-only) ──
    // These are the deterministic subset learned from two real publicize passes.
    // Audience reframing and prose quality remain in /shipshape-publicize.
    publicize::publicize_gaps(&mut gaps, repo_root, contract, fs, cmd);

    // ── GitHub community standards (read-only; failure ⇒ unknown) ──
    let community_profile = community_profile(repo_root, cmd);

    AuditReport {
        repo_root: repo_root.display().to_string(),
        maturity,
        core_complete,
        gaps,
        community_profile,
    }
}

/// Emit the tier-scaled canon (recommended) gaps — the artifacts a project of
/// this maturity is expected to carry. Cumulative: each tier adds to the one
/// below. Every canon gap is [`Severity::Recommended`], never blocking.
fn canon_gaps(
    gaps: &mut Vec<Gap>,
    repo_root: &Path,
    facts: &Facts,
    fs: &dyn Fs,
    maturity: Maturity,
) {
    // mvp adds contribution/changelog/security scaffolding + a dependency bot.
    if tier_rank(maturity) >= tier_rank(Maturity::Mvp) {
        canon_file_gap(
            gaps,
            fs,
            repo_root,
            "changelog",
            "shipshape-changelog",
            CHANGELOG_NAMES,
            "no CHANGELOG.md — mvp+ projects keep a changelog",
        );
        canon_file_gap(
            gaps,
            fs,
            repo_root,
            "contributing",
            "shipshape-contributing",
            CONTRIBUTING_NAMES,
            "no CONTRIBUTING guide — mvp+ projects onboard contributors",
        );
        canon_file_gap(
            gaps,
            fs,
            repo_root,
            "code-of-conduct",
            "shipshape-contributing",
            CODE_OF_CONDUCT_NAMES,
            "no CODE_OF_CONDUCT — mvp+ projects set community expectations",
        );
        canon_file_gap(
            gaps,
            fs,
            repo_root,
            "security-policy",
            "shipshape-security-policy",
            SECURITY_NAMES,
            "no SECURITY policy — recommended at mvp+ (required once the tool crosses a \
             threat boundary)",
        );
        if facts.dependency_bot.is_none() {
            gaps.push(canon_gap(
                "dependency-bot",
                "shipshape-ci",
                Presence::Absent,
                "no dependency-update bot (dependabot/renovate) configured — recommended at \
                 mvp+",
            ));
        }
    }

    // production adds deeper contribution + hardening scaffolding.
    if tier_rank(maturity) >= tier_rank(Maturity::Production) {
        canon_file_gap(
            gaps,
            fs,
            repo_root,
            "codeowners",
            "shipshape-contributing",
            CODEOWNERS_NAMES,
            "no CODEOWNERS — recommended at production for review routing",
        );
        canon_file_gap(
            gaps,
            fs,
            repo_root,
            "governance",
            "shipshape-contributing",
            GOVERNANCE_NAMES,
            "no GOVERNANCE.md — recommended at production",
        );
        canon_file_gap(
            gaps,
            fs,
            repo_root,
            "architecture",
            "shipshape-architecture",
            ARCHITECTURE_NAMES,
            "no ARCHITECTURE.md — offered at production (never a readiness gate)",
        );
        canon_file_gap(
            gaps,
            fs,
            repo_root,
            "pre-commit",
            "shipshape-ci",
            PRE_COMMIT_NAMES,
            "no pre-commit config — recommended at production",
        );
    }
}

/// Emit the producer-existence gaps the contract's own configuration implies.
fn producer_gaps(
    gaps: &mut Vec<Gap>,
    repo_root: &Path,
    contract: &Contract,
    facts: &Facts,
    fs: &dyn Fs,
    maturity: Maturity,
) {
    // A `fragment` changelog needs its fragment directory to exist.
    if contract.changelog.mode == ChangelogMode::Fragment {
        let dir = repo_root.join(&contract.changelog.fragment_dir);
        if !fs.is_dir(&dir) {
            gaps.push(producer_gap(
                "changelog-fragment-dir",
                "shipshape-changelog",
                Presence::Absent,
                format!(
                    "changelog.mode is 'fragment' but the fragment directory '{}' does not \
                     exist",
                    contract.changelog.fragment_dir
                ),
            ));
        }
    }

    // A declared binary distribution must cover Linux (cross-platform policy).
    cross_platform_gap(gaps, contract, maturity);

    // A registry target requires an SPDX license configured in the contract.
    let has_registry_target = contract
        .targets
        .iter()
        .any(|t| t.registry != Registry::GhReleases);
    if has_registry_target && contract.license.trim().is_empty() {
        gaps.push(producer_gap(
            "registry-license",
            "shipshape-readme",
            Presence::Absent,
            "a registry publish target is configured but the contract declares no license \
             — registries (crates.io/npm/PyPI) require an SPDX license",
        ));
    }

    // A `coverage` badge needs a coverage step in CI. Also recommended at
    // production even without the badge; emit at most one coverage gap. A
    // workflow-read failure surfaces as `Unknown`, never a false `Absent`.
    let coverage_badge = contract.health_badges.contains(&HealthBadge::Coverage);
    let coverage_expected =
        coverage_badge || tier_rank(maturity) >= tier_rank(Maturity::Production);
    if coverage_expected {
        let status = workflow_mentions(repo_root, fs, COVERAGE_TOKENS);
        if status != Presence::Present {
            let (category, detail) = if coverage_badge {
                (
                    Category::Producer,
                    "the contract enables a 'coverage' health badge but no coverage step was \
                     found in CI — the badge has no producer",
                )
            } else {
                (
                    Category::Canon,
                    "no coverage step found in CI — recommended at production",
                )
            };
            gaps.push(Gap {
                id: "coverage".to_string(),
                category,
                severity: Severity::Recommended,
                status,
                member: "shipshape-ci".to_string(),
                detail: detail.to_string(),
            });
        }
    }

    // A `scorecard` badge needs the OSSF Scorecard action wired in CI.
    if contract.health_badges.contains(&HealthBadge::Scorecard) {
        let status = workflow_mentions(repo_root, fs, SCORECARD_TOKENS);
        if status != Presence::Present {
            gaps.push(producer_gap(
                "scorecard",
                "shipshape-security-policy",
                status,
                "the contract enables a 'scorecard' health badge but no OSSF Scorecard action \
                 was found in CI — the badge has no producer",
            ));
        }
    }

    // A `ci` badge needs CI to actually exist.
    if contract.health_badges.contains(&HealthBadge::Ci) && !facts.has_ci {
        gaps.push(producer_gap(
            "ci-badge-producer",
            "shipshape-ci",
            Presence::Absent,
            "the contract enables a 'ci' health badge but no CI configuration was found — \
             the badge has no producer",
        ));
    }

    // A `license` badge needs a LICENSE file.
    if contract.health_badges.contains(&HealthBadge::License)
        && !probe(fs, repo_root, LICENSE_NAMES)
    {
        gaps.push(producer_gap(
            "license-badge-producer",
            "shipshape-readme",
            Presence::Absent,
            "the contract enables a 'license' health badge but no LICENSE file was found — \
             the badge has no producer",
        ));
    }
}

/// Query GitHub's community-standards profile for the repo (read-only).
///
/// Resolves `owner/repo` from the `origin` remote, then runs
/// `gh api repos/<owner>/<repo>/community/profile`. Any failure along the way
/// (no GitHub remote, `gh` missing, non-zero exit, unparseable JSON) degrades to
/// an unchecked profile with every field [`Presence::Unknown`] — never `Absent`.
fn community_profile(repo_root: &Path, cmd: &dyn CommandRunner) -> CommunityProfile {
    let Some(slug) = github_slug(repo_root, cmd) else {
        return unchecked_profile("no GitHub 'origin' remote could be resolved");
    };
    let path = format!("repos/{slug}/community/profile");
    let out = match cmd.run("gh", &["api", &path], repo_root) {
        Ok(out) if out.status == Some(0) => out,
        Ok(out) => {
            // A 404 (private/absent repo) or any non-zero exit is "could not
            // check", not "the files are absent".
            let reason =
                first_line(&out.stderr).unwrap_or_else(|| "gh api exited non-zero".to_string());
            return unchecked_profile(&format!("gh api failed: {reason}"));
        }
        Err(e) => return unchecked_profile(&format!("could not run gh: {e}")),
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&out.stdout) else {
        return unchecked_profile("gh api returned unparseable JSON");
    };
    // The response MUST carry a `files` object. Anything else (a `{}`, a
    // `{"message": "..."}` error body that still exited 0 through a proxy, a
    // renamed schema) is "could not check" ⇒ every field unknown, never a blanket
    // `Absent` — the outage discipline (issue: failure ⇒ unknown, never false).
    let Some(files) = json.get("files").and_then(serde_json::Value::as_object) else {
        return unchecked_profile("gh api response had no 'files' object");
    };
    // A recognized health file is a non-null object under its key; a `null` (or
    // absent) key means checked-and-absent.
    let f = |key: &str| {
        if files.get(key).is_some_and(|v| !v.is_null()) {
            Presence::Present
        } else {
            Presence::Absent
        }
    };
    // GitHub has named the security-policy field `security_policy` and (in some
    // API versions) `security`; accept either so a present SECURITY.md is not
    // misreported absent.
    let security = if matches!(f("security_policy"), Presence::Present) {
        Presence::Present
    } else {
        f("security")
    };
    CommunityProfile {
        checked: true,
        unavailable_reason: None,
        readme: f("readme"),
        license: f("license"),
        contributing: f("contributing"),
        code_of_conduct: f("code_of_conduct"),
        issue_template: f("issue_template"),
        pull_request_template: f("pull_request_template"),
        security,
    }
}

/// Resolve a `owner/repo` GitHub slug from the `origin` remote, or `None` when
/// there is no GitHub remote (or `git` failed). Parsing lives in [`crate::vcs`]
/// (shared with the release coordinator).
fn github_slug(repo_root: &Path, cmd: &dyn CommandRunner) -> Option<String> {
    let out = cmd
        .run("git", &["remote", "get-url", "origin"], repo_root)
        .ok()?;
    if out.status != Some(0) {
        return None;
    }
    crate::vcs::parse_github_slug(out.stdout.trim())
}

// ── Small builders ───────────────────────────────────────────────────────────

/// A blocking core gap (README/LICENSE/CI at the gate).
fn core_gap(id: &str, member: &str, detail: &str) -> Gap {
    Gap {
        id: id.to_string(),
        category: Category::Core,
        severity: Severity::Blocking,
        status: Presence::Absent,
        member: member.to_string(),
        detail: detail.to_string(),
    }
}

/// A recommended canon gap.
fn canon_gap(id: &str, member: &str, status: Presence, detail: &str) -> Gap {
    Gap {
        id: id.to_string(),
        category: Category::Canon,
        severity: Severity::Recommended,
        status,
        member: member.to_string(),
        detail: detail.to_string(),
    }
}

/// A recommended producer-existence gap.
fn producer_gap(id: &str, member: &str, status: Presence, detail: impl Into<String>) -> Gap {
    Gap {
        id: id.to_string(),
        category: Category::Producer,
        severity: Severity::Recommended,
        status,
        member: member.to_string(),
        detail: detail.into(),
    }
}

/// Push a canon gap for a missing file (probing the standard locations).
fn canon_file_gap(
    gaps: &mut Vec<Gap>,
    fs: &dyn Fs,
    repo_root: &Path,
    id: &str,
    member: &str,
    names: &[&str],
    detail: &str,
) {
    if !probe(fs, repo_root, names) {
        gaps.push(canon_gap(id, member, Presence::Absent, detail));
    }
}

/// An unchecked community profile — every field `Unknown`, with a reason.
fn unchecked_profile(reason: &str) -> CommunityProfile {
    CommunityProfile {
        checked: false,
        unavailable_reason: Some(reason.to_string()),
        readme: Presence::Unknown,
        license: Presence::Unknown,
        contributing: Presence::Unknown,
        code_of_conduct: Presence::Unknown,
        issue_template: Presence::Unknown,
        pull_request_template: Presence::Unknown,
        security: Presence::Unknown,
    }
}

// ── Filesystem probes ────────────────────────────────────────────────────────

/// The directories GitHub (and this audit) recognize community/health files in.
const HEALTH_DIRS: &[&str] = &["", ".github", "docs"];

/// Whether any of `names` exists as a regular file in a recognized health
/// directory (`.`, `.github`, `docs`).
fn probe(fs: &dyn Fs, repo_root: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| {
        HEALTH_DIRS.iter().any(|dir| {
            let path = if dir.is_empty() {
                repo_root.join(name)
            } else {
                repo_root.join(dir).join(name)
            };
            fs.is_file(&path)
        })
    })
}

/// Cap on a single workflow file read — a workflow this large is not real, and
/// an unbounded read would let a pathological file stall the audit.
const WORKFLOW_READ_LIMIT: usize = 1 << 20; // 1 MiB

/// Probe `.github/workflows` for any YAML file mentioning one of `tokens`
/// (case-insensitively) — the read-only producer probe for a coverage/scorecard
/// step. Tri-state, honoring the outage discipline:
///
/// - [`Presence::Present`] — a readable workflow contains a token.
/// - [`Presence::Absent`] — the directory is genuinely missing, or every YAML
///   workflow was read and none matched.
/// - [`Presence::Unknown`] — the directory or a workflow file could not be read
///   (permission/I/O error), so "no producer" cannot be asserted.
///
/// This is a substring heuristic, not a YAML parse: a token inside a comment
/// (`# TODO: add coverage`) can false-positive and an unusual action name can
/// false-negative. That is a deliberate, documented limitation — the audit only
/// *reports* the gap (never mutates), and a coarse producer signal is enough to
/// tell an agent to look; a full workflow-graph parse is out of scope here.
fn workflow_mentions(repo_root: &Path, fs: &dyn Fs, tokens: &[&str]) -> Presence {
    let dir = repo_root.join(".github/workflows");
    let entries = match fs.read_dir(&dir) {
        Ok(entries) => entries,
        // A missing directory is "checked, no producer"; any other read error
        // (permission, I/O) is "could not check".
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Presence::Absent,
        Err(_) => return Presence::Unknown,
    };
    let mut unreadable = false;
    for name in &entries {
        // Only real workflow files (`.yml`/`.yaml`) — not a stray `README.bak`.
        if !matches!(
            Path::new(name).extension().and_then(|e| e.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let path = dir.join(name);
        match fs.read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes[..bytes.len().min(WORKFLOW_READ_LIMIT)])
                    .to_lowercase();
                if tokens.iter().any(|t| text.contains(t)) {
                    return Presence::Present;
                }
            }
            // A file that vanished mid-scan is fine to skip; a genuine read
            // failure means we cannot be sure the producer is absent.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => unreadable = true,
        }
    }
    if unreadable {
        Presence::Unknown
    } else {
        Presence::Absent
    }
}

/// Emit the cross-platform gap(s) when a declared binary distribution omits an
/// OS the "installs on macOS AND Linux" policy requires — self-checked. The two
/// OSes are checked *independently*: a distribution missing both yields two gaps
/// (`distribution-linux` + `distribution-macos`), one missing yields one.
///
/// The normalizer defaults an OMITTED `platforms` to the cross-platform set
/// (macOS + Linux) and rejects an explicit empty list, so a set missing either
/// OS is only ever an *explicit* author choice; a registry-only repo has no
/// `distribution` block and is never flagged. Because the check reads the
/// contract's declared target set (not built artifacts), the wording says
/// "declares", not "builds". Severity stays [`Severity::Recommended`] (the audit
/// reserves [`Severity::Blocking`] for the gated core; the same idiom the
/// `security-policy` gap uses), but the wording escalates at production, where a
/// one-OS release is a hard gap.
fn cross_platform_gap(gaps: &mut Vec<Gap>, contract: &Contract, maturity: Maturity) {
    let production = tier_rank(maturity) >= tier_rank(Maturity::Production);
    let multi = contract.distributions.len() > 1;
    for (idx, dist) in contract.distributions.iter().enumerate() {
        // Keep the gap ids bare (`distribution-linux`/`distribution-macos`) for the
        // single-distribution case — byte-identical audit output — and disambiguate
        // per package for a monorepo so two distributions missing the same OS do not
        // collide on one id.
        let suffix = if multi {
            let key = dist.package.clone().unwrap_or_else(|| idx.to_string());
            format!(":{key}")
        } else {
            String::new()
        };
        if !dist.platforms.iter().any(|t| is_linux_triple(t)) {
            gaps.push(platform_gap(
                &format!("distribution-linux{suffix}"),
                "Linux",
                production,
            ));
        }
        if !dist.platforms.iter().any(|t| is_darwin_triple(t)) {
            gaps.push(platform_gap(
                &format!("distribution-macos{suffix}"),
                "macOS",
                production,
            ));
        }
    }
}

/// Build one cross-platform gap for a missing `os` ("Linux"/"macOS"), escalating
/// the wording at `production`.
fn platform_gap(id: &str, os: &str, production: bool) -> Gap {
    let policy = if production {
        "required by the cross-platform install policy: macOS AND Linux"
    } else {
        "cross-platform install policy: macOS AND Linux"
    };
    producer_gap(
        id,
        "shipshape-init",
        Presence::Absent,
        format!("distribution declares no {os} target — not installable on {os} ({policy})"),
    )
}

/// Whether `triple` is a desktop-Linux target-triple (`*-unknown-linux-*`, musl
/// or gnu). Rust's std desktop-Linux triples are always spelled `-unknown-linux-`,
/// so this reliably includes gnu/musl while excluding `*-linux-android` (Android
/// is not a desktop-Linux install target) and non-Linux OSes.
fn is_linux_triple(triple: &str) -> bool {
    triple.contains("-unknown-linux-")
}

/// Whether `triple` is a macOS target-triple (`*-apple-darwin`). Excludes
/// `*-apple-ios`/`*-apple-tvos` etc., which are not macOS install targets.
fn is_darwin_triple(triple: &str) -> bool {
    triple.contains("-apple-darwin")
}

/// Maturity as a monotone rank for tier comparisons (`spike < mvp < production`).
fn tier_rank(m: Maturity) -> u8 {
    match m {
        Maturity::Spike => 0,
        Maturity::Mvp => 1,
        Maturity::Production => 2,
    }
}

/// The first non-empty line of `s`, trimmed.
fn first_line(s: &str) -> Option<String> {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

// ── Artifact name tables (probed across the health directories) ──────────────

const README_NAMES: &[&str] = &["README.md", "README.rst", "README.txt", "README"];
const LICENSE_NAMES: &[&str] = &[
    "LICENSE",
    "LICENSE.md",
    "LICENSE.txt",
    "LICENCE",
    "LICENCE.md",
    "COPYING",
    "COPYING.md",
];
const CHANGELOG_NAMES: &[&str] = &["CHANGELOG.md", "CHANGELOG", "CHANGES.md", "HISTORY.md"];
const CONTRIBUTING_NAMES: &[&str] = &["CONTRIBUTING.md", "CONTRIBUTING", "CONTRIBUTING.rst"];
const CODE_OF_CONDUCT_NAMES: &[&str] = &["CODE_OF_CONDUCT.md", "CODE_OF_CONDUCT"];
const SECURITY_NAMES: &[&str] = &["SECURITY.md", "SECURITY"];
const CODEOWNERS_NAMES: &[&str] = &["CODEOWNERS"];
const GOVERNANCE_NAMES: &[&str] = &["GOVERNANCE.md", "GOVERNANCE"];
const ARCHITECTURE_NAMES: &[&str] = &["ARCHITECTURE.md", "ARCHITECTURE"];
const PRE_COMMIT_NAMES: &[&str] = &[".pre-commit-config.yaml", ".pre-commit-config.yml"];

/// CI tokens that indicate a coverage step (case-insensitive substring match).
const COVERAGE_TOKENS: &[&str] = &[
    "coverage",
    "codecov",
    "coveralls",
    "tarpaulin",
    "llvm-cov",
    "grcov",
];
/// CI tokens that indicate the OSSF Scorecard action.
const SCORECARD_TOKENS: &[&str] = &[
    "ossf/scorecard",
    "scorecard-action",
    "step-security/scorecard",
];

#[cfg(test)]
mod tests;
