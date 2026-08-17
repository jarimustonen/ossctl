---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: high
lane: bump-exec
lane_seq: 10
---

# release --bump cannot bump a single-crate manifest ([package] version)

## Description

Found 2026-08-17 during a controlled repro of release-bump-plan-uncuttable (scratch repo, single crate, no workspace). 'ossctl release cut --plan <id> --bump patch' accepted the sealed plan but the bump phase failed: 'could not find a [workspace.package] version = ... line to bump in the workspace root manifest'. bump_exec::apply_bump only rewrites the workspace-inheritance shape; a plain [package] version manifest (the most common single-crate layout, used by downstream repos) cannot use --bump at all. Reachable by any downstream user of a shipped tool; the workaround is the manual bump recipe. Fix: bump_exec falls back to rewriting [package] version in the root manifest when no [workspace.package] version exists, with the same lockstep/Cargo.lock/CHANGELOG handling. Repro detail and design context: issues/release-tag-preempts-cargo-dist/design.md (New evidence section).
