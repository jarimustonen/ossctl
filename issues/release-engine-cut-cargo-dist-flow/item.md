---
created: 2026-08-05
updated: 2026-08-05
type: feature
status: open
priority: high
epic: ossctl-phase4-build
---

# release cut can't drive the cargo-dist + homebrew flow (skip CI-delegated targets + post-tag homebrew)

_Source: crates.io/homebrew release flow — ossctl 0.1.1 self-cut_

## Description

Blocks an ENGINE-driven cut (Track B → 0.2.0). Cutting ossctl 0.1.1 had to fall back to the CI recipe (cargo-dist release.yml + publish-crates.yml) because `ossctl release cut` can't safely drive ossctl's own contract:

1. **gh-releases/cargo-dist is a partial-publish trap.** The `cargo-dist` adapter returns `AdapterError::Unsupported` in `publish()` (correct — gh-releases binaries are produced by the tag-triggered CI, not the engine). But the coordinator has NO auto-rollback and treats it as a publish-phase failure: it would publish crates.io (irreversible), then STICK forever on the Unsupported target. The coordinator must SKIP CI-delegated targets (adapter reports Unsupported/CI-delegated) rather than fail the cut.
2. **homebrew needs a post-tag tarball.** The homebrew url is the GitHub tag-archive (`.../archive/refs/tags/vX.Y.Z.tar.gz`), which only exists AFTER the tag-once phase; the engine threads `sha256: None` and opens a DRAFT PR needing a hand-filled hash (see homebrew-adapter-first-formula 'known limitation'). Needs a post-tag distribution phase (or an ossctl-built/uploaded source tarball whose bytes it controls) so homebrew gets a correct sha256.

RELATED: `release-cut-multi-target-ecosystem` — ossctl's own contract has TWO crates.io targets (ossctl-core + ossctl) which `release cut` also rejects ('two targets resolve to the same journal id rust'). All three must resolve for an engine-driven 0.2.0 cut. See AGENTS.md 'Operating policy' for the current CI-recipe process this replaces.
