---
created: 2026-08-04
updated: 2026-08-07
type: improvement
status: done
priority: normal
related: ['@contract-cannot-model-cargo-dist-release']
commits:
- hash: 1d24911
  summary: capture unknown distribution sub-keys in extra_fields
- hash: '2221631'
  summary: review fixes — keep Eq, version the nested warning, add coexistence + drift-guard tests
closed: 2026-08-07
---

# Nested distribution block: extra_fields forward-compat capture

_Source: /llm-review of contract-cannot-model-cargo-dist-release_

## Description

Deferred spin-off from the /llm-review of `contract-cannot-model-cargo-dist-release`. The top-level `Contract` captures unknown keys via an `extra_fields` forward-compat mechanism, but the new nested `Distribution` (and its `gh_releases`/`installers`/`homebrew_tap` sub-structs) may not. Add the same `extra_fields` capture to the nested distribution structs so an older ossctl reading a newer contract preserves (rather than drops) unknown distribution sub-keys.
