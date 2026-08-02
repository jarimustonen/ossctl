---
created: 2026-08-02
updated: 2026-08-02
type: task
status: in-progress
priority: normal
epic: ossctl-phase4-build
commits:
- hash: b40a843
  summary: add CHANGELOG, CONTRIBUTING, SECURITY, dependabot.yml
---

# Close 4 recommended readiness gaps (changelog, contributing, security, dep-bot)

## Description

From 'ossctl audit' (stint #9), gated core is complete; these 4 recommended gaps to close before the 0.1.0 cut. Code-of-conduct DELIBERATELY EXCLUDED (maintainer decision: no community yet; add later when external contributors arrive).

DELIVER (dogfood the bundled /oss-* skills via 'ossctl skill print <name>' for authoring guidance, then follow ossctl's own contract):
1. CHANGELOG.md (oss-changelog) — Keep-a-Changelog skeleton with an [Unreleased] section prepared for 0.1.0. config changelog.mode=curated, source=issuectl-trailers. NOTE: commits use 'Issue: <slug>' trailers, NOT Refs-Issue/Fixes-Issue, so 'issuectl changelog' likely returns nothing — hand-curate the 0.1.0 entry from the closed issues / git history if so.
2. CONTRIBUTING.md (oss-contributing) — build steps, the green gate (cargo fmt --check, clippy -D warnings, test, build), issuectl workflow, PR expectations. DO NOT emit a code of conduct (excluded).
3. SECURITY.md (oss-security-policy) — coordinated-disclosure policy sized to the threat surface. PREFER GitHub private vulnerability reporting as the channel; do NOT publish a personal email address unless one is already public in the repo. Supported versions table (0.1.x).
4. .github/dependabot.yml (oss-ci) — dependabot for the cargo ecosystem (weekly), matching config dependency_bot=dependabot. Keep it minimal; do NOT rewrite the existing ci.yml.

Re-run 'cargo run -q --bin ossctl -- audit' at the end — the 4 gaps should be gone (code-of-conduct may remain listed as recommended; that's expected/accepted).

Green gate: cargo fmt --check, clippy -D warnings, test --workspace, build --workspace (these are docs+config so should stay green trivially). Run /llm-review + /assess-findings on the diff before merge.
