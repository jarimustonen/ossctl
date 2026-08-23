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
