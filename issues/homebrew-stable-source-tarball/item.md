---
created: 2026-08-05
updated: 2026-08-05
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@release-engine-cut-cargo-dist-flow']
---

# homebrew: own the source tarball bytes instead of GitHub unstable tag archive

_Source: review of release-engine-cut-cargo-dist-flow_

## Description

The post-tag dist phase computes the homebrew sha256 by fetching+hashing GitHub auto-generated tag archive. Sanctioned by the issue, but GitHub does not guarantee byte-stability of generated archives; Homebrew formulae have broken from this historically. Long-term: build a deterministic source tarball locally, hash in-process (ossctl-core vendors a pure-Rust SHA-256 in release/plan.rs::sha256), upload as a GitHub Release asset, point the formula at that immutable URL. Deferred, not a blocker. Surfaced by /llm-review.
