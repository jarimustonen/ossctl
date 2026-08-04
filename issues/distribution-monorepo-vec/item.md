---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: open
priority: normal
related: ['@contract-cannot-model-cargo-dist-release']
---

# Monorepo distribution: Vec<Distribution> with per-package association

_Source: /llm-review of contract-cannot-model-cargo-dist-release_

## Description

Deferred spin-off from the /llm-review of `contract-cannot-model-cargo-dist-release`. The new `distribution` block is a single `Option<Distribution>` on `Contract`, which models a single-artifact repo. A monorepo can ship multiple independently-distributed binaries (each with its own gh-releases/installer/tap). Extend the model to `Vec<Distribution>` with a way to associate each distribution with the package/target it belongs to, without breaking the single-distribution common case.
