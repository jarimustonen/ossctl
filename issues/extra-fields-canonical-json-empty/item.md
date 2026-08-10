---
created: 2026-08-07
updated: 2026-08-07
type: improvement
status: open
priority: normal
related: ['@distribution-extra-fields']
---

# extra_fields: decide skip_serializing_if vs always-present {} in canonical JSON

## Description

_Source: /llm-review of distribution-extra-fields (deferred, unanimous — a holistic shape decision, not a distribution-local one)._

All four reviewers noted that both `Contract.extra_fields` and (now) `Distribution.extra_fields` are ALWAYS serialized, so every distribution block gains `"extra_fields": {}` even when empty. This is a (small) canonical-JSON shape change for existing distribution contracts.

In distribution-extra-fields it was kept always-present ON PURPOSE — to mirror the ESTABLISHED top-level `extra_fields`, which is itself always-present (asserted by `serializes_to_schema_v4_shape`). The task prioritized consistency with the existing mechanism over introducing a divergent `skip_serializing_if` on only the nested field. No `schema_version` bump: the addition is additive under the migration rule (downstream members key-access specific fields; an added key is tolerated), consistent with how top-level `extra_fields` was introduced under schema_version 1.

Decision to make (holistically, for BOTH fields at once so they stay symmetric):
- **Option A** — add `#[serde(skip_serializing_if = "serde_json::Map::is_empty")]` to BOTH `extra_fields` fields, so an empty map is absent from canonical JSON and the "additive = absent-by-default" rule holds literally. Update `serializes_to_schema_v4_shape` (which currently asserts the key is present) and any fixtures.
- **Option B** — keep always-present `{}` as the documented schema-v1 canonical shape and clarify the `KNOWN_SCHEMA_VERSION` doc that `extra_fields: {}` is part of the shape.

Do NOT change one field without the other — asymmetry between top-level and nested is worse than either option.

## Decision (Jari, 2026-08-10) — Option A: omit when empty

**Chosen: A — `skip_serializing_if` so an empty `extra_fields` map is absent from canonical JSON.**
Apply to BOTH fields symmetrically (`Contract.extra_fields` AND `Distribution.extra_fields`) so the
"additive = absent-by-default" rule holds literally. Update `serializes_to_schema_v4_shape` (which
currently asserts the key is present) and any affected fixtures/golden vectors. Consider whether this
is a canonical-shape change needing a `schema_version` note — but since it only removes an always-empty
`{}`, existing populated contracts are unaffected; justify the call. NOTE: `distribution-monorepo-vec`
already moved to schema_version 2 (`distributions` array) — read the current shape.
