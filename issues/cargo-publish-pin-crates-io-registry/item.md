---
created: 2026-08-05
updated: 2026-08-05
type: bug
status: in-progress
priority: normal
epic: ossctl-phase4-build
commits:
- hash: c605eaa
  summary: pin cargo adapter to crates.io, reject other registries
---

## Description

Surfaced by the `/llm-review` of ADR-0004 (`cargo-adapter-multitarget-double-publish`). Pre-existing; ADR-0004 did not touch it.

## Problem

The cargo adapter's `build()`/`dry_run()`/`publish()` emit bare `cargo package -p <pkg>` / `cargo publish -p <pkg>` (+ `--dry-run`). Cargo honors ambient registry configuration (`registry.default`, `[registry]` in `.cargo/config.toml`, `CARGO_REGISTRY_DEFAULT`). So on a misconfigured host/runner, `cargo publish` could push to a **different** registry while the adapter probes crates.io, waits on crates.io, and records a crates.io receipt + URL — a silent wrong-destination publish with a false receipt.

## Fix

Pin every cargo publish/package/dry-run to crates.io: `cargo publish --registry crates-io -p <pkg>` (and same for `cargo package`). The registry should be derived from `AdapterTarget.target.registry`; reject any target whose registry is not `Registry::CratesIo` in this adapter. Also fold in the latent `Ecosystem::Rust.as_str()` / `remote_url` hard-coding (they ignore `t.target.registry`) — harmless today (crates.io is the only rust registry) but the same alt-registry trap.

Refs-Issue: cargo-adapter-multitarget-double-publish
