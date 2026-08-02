---
created: 2026-08-02
updated: 2026-08-02
type: task
status: open
priority: high
epic: ossctl-phase4-build
---

# Make ossctl publishable to crates.io as `ossctl`

## Description

Pre-cut readiness surfaced by the /oss-init dogfood (stint #9). Blocks the real 0.1.0 release cut.

Scope (decided with Jari):
1. RENAME + PUBLISH: publish the CLI crate as `ossctl` (so `cargo install ossctl` works) and set `publish = true` on BOTH workspace crates. Currently crates/ossctl-cli/Cargo.toml and crates/ossctl-core/Cargo.toml both carry `publish = false`, and there is no crate literally named `ossctl`. Rename the CLI package name from `ossctl-cli` to `ossctl` (keep the [[bin]] name `ossctl`). Prefer renaming ONLY the package `name` field to minimize churn; renaming the directory crates/ossctl-cli -> crates/ossctl is optional and would ripple through many doc paths (AGENTS.md hot-file list etc.) so weigh the cost. ossctl-core keeps its name; just flip publish=true (crates.io requires published deps).
2. LICENSE: add a MIT LICENSE file at the repo root (manifest already declares license = MIT; the text file is missing).
3. CONFIG: update OSS-RELEASE.md crates.io target package from `ossctl-cli` to `ossctl` to match the rename. Re-validate with 'cargo run --bin ossctl -- contract validate' AND check-oss-release.py (both must pass).

Green gate: cargo fmt --check, clippy -D warnings, test --workspace, build --workspace. Run /llm-review + /assess-findings before merging (touches production manifests + release config).
