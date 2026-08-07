---
created: 2026-08-07
updated: 2026-08-07
type: improvement
status: in-progress
priority: normal
related: ['@distribution-extra-fields']
---

# extra_fields capture: shared hardening (non-string keys, reserved-key round-trip, dedupe)

## Description

_Source: /llm-review of distribution-extra-fields (deferred, systemic — applies to BOTH the top-level `Contract.extra_fields` scan in `build` and the nested `parse_distribution` scan)._

The forward-compat unknown-key capture has three shared gaps flagged by all reviewers. They pre-date this change (the top-level scan has them too) and were left out of scope to avoid touching the established top-level shape:

1. **Non-string YAML keys silently dropped.** Both scans use `if let Value::String(key) = k`. A non-string mapping key (`42: x`, `true: y`) is neither captured under `extra_fields` nor warned/errored — it vanishes, violating the "unknown keys are never dropped" guarantee. Decide: coerce via `yaml_display` + warn, or reject as a structural error. Apply to both scans.

2. **Canonical output is not valid normalizer input (reserved-key / recursive-nesting).** `extra_fields` and `warnings` are not in `KNOWN_KEYS`/`KNOWN_DISTRIBUTION_KEYS`. If serialized canonical JSON is ever re-fed to the normalizer, `extra_fields` is itself captured as an unknown key → `extra_fields.extra_fields` nesting on each pass. Either document+test that canonical output is not normalizer input, or reserve/parse the canonical metadata keys explicitly.

3. **Duplicated scan logic is a drift hazard.** The capture+warn block is near-verbatim in `build` and `parse_distribution`. The distribution warning originally omitted `schema_version` (fixed in this issue's PR) — exactly the drift a shared `capture_unknown_fields(mapping, known_keys, schema_version, p) -> Map` helper would prevent. Extract one helper both call sites use.

Also (maintenance): add a doc note that `KNOWN_KEYS`/`KNOWN_DISTRIBUTION_KEYS` MUST stay in sync with the struct fields (or derive the known set from serde). A test now exercises every distribution known key (`distribution_all_known_keys_has_empty_extra_fields`); consider the same for the top level.
