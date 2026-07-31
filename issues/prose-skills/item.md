---
created: 2026-07-25
updated: 2026-07-31
type: feature
status: done
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@audit-command']
closed: 2026-07-31
---

# Author the prose /oss-* members + the /oss-release orchestrator (bundled)

## Description

Author the skill-side (non-binary) family members per design.md §2 roster + the ADR-0001 boundary table, bundled here and version-pinned: /oss-readme (README+LICENSE, MIT default), /oss-ci (workflow YAML), /oss-changelog (wraps issuectl changelog), /oss-contributing, /oss-security-policy (threat-gated), /oss-architecture (opt-in), and /oss-release (the orchestrator/router: mode detection, sequencing, hands off to release plan/cut). Each reads config via 'ossctl contract show' and aborts on non-zero. Substantial members via /worktree-make-skill, small via /skill-creator, mirroring the family design's §9.2 split. Likely splits into per-member issues when picked up. Blocked by contract-command + audit-command.
