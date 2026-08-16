---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: obsolete
priority: high
closed: 2026-08-16
---

# release cut publishes the manifest version, not --version — no bump, no mismatch refusal

## Description

## Symptom

`ossctl release cut --plan <id> --version 0.1.0` on a workspace whose Cargo.toml still declared
`version = "0.0.0"` **published `project-canon-core@0.0.0` to crates.io** (permanent), while
the run journal labelled the target `0.1.0`. The publish phase ran `cargo publish -p <crate>`,
which uses the manifest version — so the engine published 0.0.0 and recorded it as 0.1.0.

## Impact

Permanent mis-publish. crates.io versions cannot be deleted or reused, so a stray `0.0.0` was
burned. `release verify` then reports the intended `0.1.0` as `missing` (registry has 0.0.0),
and the run cannot be reconciled.

## Expected

One of: (a) the engine bumps the workspace/crate version to `--version` before build/publish
(then commits/tags it), or (b) `release plan`/`cut` **refuses** when `--version` != the
manifest version, with a clear error, instead of silently publishing the manifest version.

## Evidence

project-canon 0.1.x release, 2026-08-16. Plan sealed `version: 0.1.0` against a tree at
`0.0.0`; cut published core@0.0.0. Repro: any repo where manifest version != --version.

## Env
ossctl 0.2.2, cargo-publish adapter, rust workspace.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Obsolete after later release-engine changes: --version was removed, manifest version is the single source of truth, and --bump owns version changes. Refile a new issue if a current 0.5.x cut can reproduce a wrong-version publish.
