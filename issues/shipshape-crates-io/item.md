---
created: 2026-08-23
updated: 2026-08-23
type: bug
status: fixed
priority: high
lane: release-engine
closed: 2026-08-23
---

# Shipshape crates.io package name is already owned

## Description

The first Shipshape 0.11.0 cut (run 01M0QJKSEJZ0Z3JQGN0Q9ADE0Y, plan 5fece070...) published shipshape-core 0.11.0, then crates.io rejected package shipshape with 403 because the name belongs to an unrelated Docker crate (shipshape 0.1.1). No tag, GitHub Release, Homebrew formula, or default-branch advance occurred. Preserve the canonical executable/product name shipshape, migrate the Cargo registry package coordinate to the available shipshape-cli name, and preserve durable run/plan compatibility. The interrupted old run must be explicitly abandoned after the replacement path is ready; the replacement cut must reconcile the already-published shipshape-core 0.11.0 rather than duplicate it.

## Resolution

### 2026-08-23T16:08:31Z · @issuectl

Fixed by moving the registry package to shipshape-cli, adding a non-published shipshape cargo-dist naming wrapper, pre-bumping a resumable 0.11.0 recovery tree, and guarding the temporary core omission until verified post-cut cleanup. Full green gate and pinned cargo-dist package/build verification pass.
