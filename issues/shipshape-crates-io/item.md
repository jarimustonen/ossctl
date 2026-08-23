---
created: 2026-08-23
updated: 2026-08-24
type: bug
status: fixed
priority: high
lane: release-engine
commits:
- hash: ed3df46
  summary: target registry CLI package in CI and release workflows
- hash: 0e14c65
  summary: restore steady-state crate publishing
closed: 2026-08-24
---

# Shipshape crates.io package name is already owned

## Description

The first Shipshape 0.11.0 cut (run 01M0QJKSEJZ0Z3JQGN0Q9ADE0Y, plan 5fece070...) published shipshape-core 0.11.0, then crates.io rejected package shipshape with 403 because the name belongs to an unrelated Docker crate (shipshape 0.1.1). No tag, GitHub Release, Homebrew formula, or default-branch advance occurred. Preserve the canonical executable/product name shipshape, migrate the Cargo registry package coordinate to the available shipshape-cli name, and preserve durable run/plan compatibility. The interrupted old run must be explicitly abandoned after the replacement path is ready; the replacement cut must reconcile the already-published shipshape-core 0.11.0 rather than duplicate it.

## Resolution

### 2026-08-23T16:08:31Z · @issuectl

Fixed by moving the registry package to shipshape-cli, adding a non-published shipshape cargo-dist naming wrapper, pre-bumping a resumable 0.11.0 recovery tree, and guarding the temporary core omission until verified post-cut cleanup. Full green gate and pinned cargo-dist package/build verification pass.

### 2026-08-23T16:31:53Z · @issuectl

Reopen regression resolved: CI lockstep now selects Cargo package shipshape-cli, the tag-triggered crates.io workflow skips the deliberately non-publishable recovery core via Cargo metadata and publishes shipshape-cli, and the workspace-wide lockstep test guards the registry/package boundary. Exact failing command and complete green gate passed; four-model review findings were assessed and confirmed localized fixes applied.

### 2026-08-23T21:06:25Z · @issuectl

Post-cut cleanup complete after verified v0.11.0 recovery: shipshape-core is publishable and first in the crates.io target order, active recovery-only workflow/contract behavior is removed, facts/normalizer/planner regressions cover both crates and the exact pin, and the full green gate passed. Engine run 01M0QQMW8Y6SWRR2G383M0KVJX remains honestly abandoned.



## Reopen Notes — 2026-08-23

_Add rationale for reopening here._

## Comments

### 2026-08-23T16:11:28Z · @conductor

Reopened after main CI run 32650811109: skill↔CLI lockstep still invokes `cargo test --locked -p shipshape --test skill --test skill_lockstep`; after the registry package rename those tests belong to package `shipshape-cli`. All other CI jobs passed. Release remains halted until the workflow is corrected and main CI is green.

### 2026-08-23T20:45:50Z · @conductor

Post-cut cleanup is now unblocked. v0.11.0 recovery verified crates.io shipshape-core + shipshape-cli, GitHub Release exactly three supported platforms, and Homebrew formula; old four-platform run remains abandoned. Restore shipshape-core publish=true and the first cargo-publish target for future releases, remove temporary recovery-only contract wording, run full gate/CI, then close.


## Reopen Notes — 2026-08-23

_Add rationale for reopening here._
