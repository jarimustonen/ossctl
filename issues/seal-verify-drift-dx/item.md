---
created: 2026-08-04
updated: 2026-08-16
type: improvement
status: duplicate
priority: normal
related: ['@contract-cannot-model-cargo-dist-release']
closed: 2026-08-16
---

# SEAL_VERSION drift DX: make the seal bump + golden-vector update ergonomic

_Source: /llm-review of contract-cannot-model-cargo-dist-release_

## Description

Deferred spin-off from the /llm-review of `contract-cannot-model-cargo-dist-release`. Adding the `distribution` block bumped `SEAL_VERSION` 1→2 because the content-addressed plan_id pre-image embeds the whole serialized `Contract`, which forced a manual golden-vector update. Any future field on `Contract` will trigger the same churn. Improve the developer experience: a clearer failure message when the seal drifts, and/or a tooling path to regenerate the golden vector deliberately (not by hand). See the whole-Contract-hash question in the sibling `plan-preimage-projection` issue — the two are related.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Subsumed by the broader plan preimage and seal ergonomics question.
