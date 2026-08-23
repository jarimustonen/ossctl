---
created: 2026-08-23
updated: 2026-08-23
type: bug
status: open
priority: high
lane: release-engine
---

# Shipshape crates.io package name is already owned

## Description

The first Shipshape 0.11.0 cut (run 01M0QJKSEJZ0Z3JQGN0Q9ADE0Y, plan 5fece070...) published shipshape-core 0.11.0, then crates.io rejected package shipshape with 403 because the name belongs to an unrelated Docker crate (shipshape 0.1.1). No tag, GitHub Release, Homebrew formula, or default-branch advance occurred. Preserve the canonical executable/product name shipshape, migrate the Cargo registry package coordinate to the available shipshape-cli name, and preserve durable run/plan compatibility. The interrupted old run must be explicitly abandoned after the replacement path is ready; the replacement cut must reconcile the already-published shipshape-core 0.11.0 rather than duplicate it.

## Resolution

### 2026-08-23T16:08:31Z · @issuectl

Fixed by moving the registry package to shipshape-cli, adding a non-published shipshape cargo-dist naming wrapper, pre-bumping a resumable 0.11.0 recovery tree, and guarding the temporary core omission until verified post-cut cleanup. Full green gate and pinned cargo-dist package/build verification pass.

## Reopen Notes — 2026-08-23

_Add rationale for reopening here._

## Comments

### 2026-08-23T16:11:28Z · @conductor

Reopened after main CI run 32650811109: skill↔CLI lockstep still invokes `cargo test --locked -p shipshape --test skill --test skill_lockstep`; after the registry package rename those tests belong to package `shipshape-cli`. All other CI jobs passed. Release remains halted until the workflow is corrected and main CI is green.
