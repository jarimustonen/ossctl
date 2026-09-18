---
created: 2026-09-18
updated: 2026-09-18
type: chore
status: done
priority: normal
provenance: chat
source_ref: taskfleet:01m2sm2d976zvp49dvps98s1yt/task:cargo-dist-0.33.0
originating_run: 01m2sm2d976zvp49dvps98s1yt
originating_run_kind: spinoff
closed: 2026-09-18
---

# Upgrade cargo-dist pin to 0.33.0

## Description

Upgrade this repository's cargo-dist configuration and Shipshape's generated default from the active pinned versions to stable 0.33.0. Regenerate the release workflow with the verified disposable 0.33.0 binary while preserving distribution semantics, and update active assertions without changing historical recovery evidence.

## Acceptance Criteria

- [x] `dist-workspace.toml`, generated release workflow, and Shipshape generator output pin 0.33.0.
- [x] Active tests and documentation assertions use 0.33.0; historical 0.28.2 recovery evidence remains intact.
- [x] cargo-dist 0.33.0 CI check and JSON plan pass.
- [x] Repository green gate passes.

## Resolution

### 2026-09-18T06:47:28Z · @issuectl

Upgraded the repository and downstream generator pin to cargo-dist 0.33.0, regenerated the workflow with the verified disposable binary, and passed cargo-dist checks plus the full Rust green gate.
