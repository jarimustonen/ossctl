---
created: 2026-09-18
updated: 2026-09-18
type: chore
status: in-progress
priority: normal
provenance: chat
source_ref: taskfleet:01m2sm2d976zvp49dvps98s1yt/task:cargo-dist-0.33.0
originating_run: 01m2sm2d976zvp49dvps98s1yt
originating_run_kind: spinoff
---

# Upgrade cargo-dist pin to 0.33.0

## Description

Upgrade this repository's cargo-dist configuration and Shipshape's generated default from the active pinned versions to stable 0.33.0. Regenerate the release workflow with the verified disposable 0.33.0 binary while preserving distribution semantics, and update active assertions without changing historical recovery evidence.

## Acceptance criteria

- [x] `dist-workspace.toml`, generated release workflow, and Shipshape generator output pin 0.33.0.
- [x] Active tests and documentation assertions use 0.33.0; historical 0.28.2 recovery evidence remains intact.
- [x] cargo-dist 0.33.0 CI check and JSON plan pass.
- [x] Repository green gate passes.
