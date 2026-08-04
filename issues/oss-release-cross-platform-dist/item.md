---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: open
priority: high
epic: ossctl-phase4-build
blocked_by: ['@distribution-cross-platform-targets']
---

# Downstream release-infra generation must default to cross-platform (Mac+Linux) binaries

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

REQUIREMENT: programs that USE ossctl must get Linux binaries too, not just macOS. Ensure the /oss-* family's release-infra generation — the cargo-dist config (dist-workspace.toml) and tag-triggered release.yml written for a downstream project (owned by the release-cut/oss-release skill prose in crates/ossctl-cli/skills/) — defaults its target set to the CROSS-PLATFORM set from the contract's distribution targets (Mac aarch64/x86_64 + Linux musl aarch64/x86_64) and enables the shell installer. Mirror what ossctl's own dist-workspace.toml now does. Depends on distribution-cross-platform-targets. If the downstream release.yml generator does not exist yet, this issue at minimum makes the cross-platform target set the documented default the generator (current or future) reads from the contract.
