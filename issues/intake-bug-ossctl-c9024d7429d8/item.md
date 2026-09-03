---
created: 2026-09-03
updated: 2026-09-03
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:project-canon-070-missing-dist-preflight
---

# Release dry-run misses required dist executable

## Description

Release dry-run misses required dist executable

`shipshape release cut --plan ffb56e4a2c440a5061869107a8501e6eea9dcdb3a8babd809476bf8a902e55f5 --json` started release run `01M1GBZC3ZGAGMSCQ6THT6VAR8` for project-canon 0.7.0. The `dry_run` phase completed successfully for all four targets, including the cargo-dist-backed GitHub Release and binary targets. The following build phase then failed immediately:

`build-phase failed on target rust:project-canon-cli:gh-releases: cannot run dist build: No such file or directory (os error 2)`

Expected: preflight or `dry_run` should resolve and execute-check every required external program before creating/progressing a release run, and fail with an actionable diagnostic naming the required cargo-dist version or installation remedy.

Observed impact: the sealed run was left in progress and required installing cargo-dist 0.28.2 into a disposable prefix and calling `shipshape release resume`. No target had published, and the same journaled run eventually completed safely. This is a deterministic dependency-preflight gap, not a release corruption.
