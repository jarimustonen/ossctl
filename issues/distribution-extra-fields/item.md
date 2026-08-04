---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: open
priority: normal
related: ['@contract-cannot-model-cargo-dist-release']
---

# Nested distribution block: extra_fields forward-compat capture

_Source: /llm-review of contract-cannot-model-cargo-dist-release_

## Description

Deferred spin-off from the /llm-review of `contract-cannot-model-cargo-dist-release`. The top-level `Contract` captures unknown keys via an `extra_fields` forward-compat mechanism, but the new nested `Distribution` (and its `gh_releases`/`installers`/`homebrew_tap` sub-structs) may not. Add the same `extra_fields` capture to the nested distribution structs so an older ossctl reading a newer contract preserves (rather than drops) unknown distribution sub-keys.
