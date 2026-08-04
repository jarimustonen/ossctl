---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: in-progress
priority: high
epic: ossctl-phase4-build
blocked_by: ['@distribution-cross-platform-targets']
---

# oss-readme must emit cross-platform install snippets (shell installer + gh-release binaries)

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

REQUIREMENT: downstream READMEs must show how to install on BOTH macOS and Linux. Today `oss-readme` (crates/ossctl-cli/skills/oss-readme/SKILL.md) emits install snippets only from registry `targets[]` (cargo install / brew install). When the contract has a `distribution` block it must ALSO emit: a shell-installer snippet (`curl -LsSf .../<pkg>-installer.sh | sh` — works on macOS+Linux), a GitHub-Release prebuilt-binary path, and a note that the platform coverage includes Linux + macOS (from the distribution's cross-platform targets, issue distribution-cross-platform-targets). Note brew also works on Linuxbrew. Depends on distribution-cross-platform-targets for the target list.
