---
created: 2026-08-12
updated: 2026-08-12
type: feature
status: open
priority: high
labels: [release, rust]
---

# release engine: support dependency-ordered multi-crate Rust workspace publish + version bump (retire hand-cut releases)

## Description

GOAL: make `ossctl release plan/cut` able to cut orchestratectl's releases so we can RETIRE hand-cutting (0.1.1-0.1.6 were all hand-cut; `ossctl release list` shows only an abandoned v0.1.0 run — the engine has never successfully cut a release for this repo).

## Observed gap (ossctl 0.2.2, repo ~/Sources/orchestratectl)
`ossctl release plan --version 0.1.6 --json` for a two-crate workspace (crates/octl-core lib + crates/octl-cli bin `orchestratectl`, where the CLI depends on `octl-core = "=<version>"`) produced an INCOMPLETE plan:
- targets: **only** `{ecosystem: rust, package: orchestratectl, registry: crates.io}` — `octl-core` is NOT a target, so a cut would `cargo publish orchestratectl` while `octl-core@<new-version>` does not yet exist on crates.io → publish fails on the `=<version>` pin.
- **no version-bump phase**: head_sha was the pre-bump commit (Cargo.toml still at the old version); phases were [dry-run-all, build-all, publish-all, tag, dist]. Nothing bumps the workspace `version`, the octl-cli `octl-core = "=X"` pin, or Cargo.lock.
- `homebrew_tap: null` even though OSS-RELEASE.md's distribution + the per-tool tap `jarimustonen/orchestratectl` exist.

## What the engine should do for a Rust workspace release
1. **Derive dependency-ordered member publish** from the workspace graph: publish path-dependency crates before their dependents (octl-core → orchestratectl), waiting for each to be available on the registry (as `cargo publish` already does within a crate). Both members are `publish = true`.
2. **Own the version bump** as a plan phase: set the workspace `[workspace.package] version`, rewrite intra-workspace `=<version>` pins (octl-cli's `octl-core = "=X"`), refresh Cargo.lock, and finalize CHANGELOG (`[Unreleased]` → dated) — content-addressed like the rest of the plan.
3. **Regenerate version-embedding test snapshots** as part of the bump (orchestratectl has insta `envelope_snapshots__version_{text,json,jsonl}` that embed the version + per-skill cli_version; a bump restales them and reds CI). Either regenerate + strip insta's volatile `assertion_line:` header, or provide a documented hook the repo runs.
4. **Carry homebrew_tap** from the contract's distribution block into the plan (tag → cargo-dist Release CI builds binaries + updates the tap).

## Evidence
`ossctl release list --json` → only abandoned v0.1.0. `ossctl release plan --version 0.1.6 --json` → the single-target/no-bump plan above. Hand-cut reference (what the engine must reproduce): the `release: vX.Y.Z` commits on orchestratectl main + TODO.md 'RELEASE STATE'.

## Done
`ossctl release cut` produces a correct, coherent orchestratectl release end-to-end (both crates published in order at the bumped version, snapshots green, tag pushed, tap updated) — verified on a real cut — so the repo's AGENTS.md 'the /oss-release skill orchestrates the whole thing' becomes true and hand-cutting is retired.
