---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: fixed
priority: high
epic: ossctl-phase4-build
blocked_by: ['@distribution-cross-platform-targets']
closed: 2026-08-04
---

# Downstream release-infra generation must default to cross-platform (Mac+Linux) binaries

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

REQUIREMENT: programs that USE ossctl must get Linux binaries too, not just macOS. Ensure the /oss-* family's release-infra generation — the cargo-dist config (dist-workspace.toml) and tag-triggered release.yml written for a downstream project (owned by the release-cut/oss-release skill prose in crates/ossctl-cli/skills/) — defaults its target set to the CROSS-PLATFORM set from the contract's distribution targets (Mac aarch64/x86_64 + Linux musl aarch64/x86_64) and enables the shell installer. Mirror what ossctl's own dist-workspace.toml now does. Depends on distribution-cross-platform-targets. If the downstream release.yml generator does not exist yet, this issue at minimum makes the cross-platform target set the documented default the generator (current or future) reads from the contract.

## Resolution

**Outcome:** The actual release-infra generator does **not exist yet** — neither
as engine code nor as skill prose. Verified: `crates/ossctl-core/src/release/`
consumes only `distribution.homebrew_tap` (threaded in `plan.rs` for the Homebrew
adapter); nothing generates `dist-workspace.toml` or `release.yml` from
`distribution.platforms`. The `oss-ci` skill attributes `release.yml` to a
`/oss-release-cut` skill that isn't a separate dir — the release-cut lives in the
`oss-release` orchestrator, which hands off to `ossctl release cut` (the engine).

**Change (documented default, per the issue's fallback clause):** Added a
"Release-infra generation — cross-platform by default (Mac + Linux)" subsection
to `crates/ossctl-cli/skills/oss-release/SKILL.template.md` (Cut-release mode). It
mandates mapping `distribution.platforms` → cargo-dist `[dist] targets`
(cross-platform default incl. Linux musl when omitted; **never** a macOS-only
matrix) and `distribution.installers` → `[dist] installers` (ensure `shell` for
the Unix curl-installer covering Mac+Linux), mirrors `ossctl`'s own
`dist-workspace.toml`, keeps `dist generate` as the sole author of `release.yml`,
and references the `AGENTS.md` cross-platform hard requirement. A `Status (engine
gap)` note records that the engine doesn't yet generate this and that the mapping
is the binding default the generator (current or future) reads from the contract.

**Spin-off filed:** `release-engine-dist-config-generator` — build the code that
actually emits the config from the contract.

**Green gate:** `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace` (256 core + skill_lockstep
7, all pass), `cargo build --workspace` — all green. Prose-only change (no logic),
so `/llm-review` was not required.
