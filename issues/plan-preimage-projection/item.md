---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: open
priority: normal
related: ['@contract-cannot-model-cargo-dist-release']
---

# plan_id pre-image: hash a stable projection instead of the whole Contract

_Source: /llm-review of contract-cannot-model-cargo-dist-release_

## Description

Deferred spin-off from the /llm-review of `contract-cannot-model-cargo-dist-release`. The content-addressed `plan_id` pre-image currently embeds the ENTIRE serialized `Contract`, so any additive field (like the new `distribution` block) changes the seal even when it is release-irrelevant, forcing a SEAL_VERSION bump + golden-vector update. Consider hashing a stable, release-relevant PROJECTION of the contract instead of the whole struct, so cosmetic/forward-compat fields don't perturb the plan identity. Related to `seal-verify-drift-dx`.
