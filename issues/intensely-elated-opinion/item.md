---
created: 2026-08-07
updated: 2026-08-07
type: improvement
status: open
priority: normal
---

# extra_fields nested non-string keys collapse in yaml_to_json (never-drop gap)

## Description

_Source: /llm-review of extra-fields-capture-hardening (deferred, pre-existing)._

The top-level and `distribution` capture scans now REJECT non-string mapping keys (extra-fields-capture-hardening). But `yaml_to_json` — which converts a preserved VALUE to JSON — still coerces non-string keys inside a nested mapping via `yaml_display`:

```rust
Value::Mapping(m) => {
    for (k, val) in m {
        let key = match k {
            Value::String(s) => s.clone(),
            other => yaml_display(other), // collapses 42 and "42" → "42"; every list → "<list>"
        };
        obj.insert(key, yaml_to_json(val)); // Map::insert overwrites → silent drop
    }
}
```

So a preserved field whose value contains non-string keys still silently drops values:
```yaml
future_x:
  42: a
  "42": b   # overwrites — one value lost, no warning
```

This is PRE-EXISTING (predates the shared-helper work) and broader than the two top-level scans that issue scoped, so it was deferred. Options: reject non-string keys anywhere in preserved content (thread a Result + path through `yaml_to_json`), or a lossless/collision-safe key encoding. Related: @extra-fields-capture-hardening.
