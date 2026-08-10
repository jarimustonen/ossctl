---
created: 2026-08-07
updated: 2026-08-10
type: improvement
status: in-progress
priority: normal
commits:
- hash: 355ceca
  summary: JSON-encode user keys/values in normalizer diagnostics (log-injection hardening) + tests
---

# Diagnostic log-injection: unescaped keys in normalizer warning/error text

## Description

Source: /llm-review of extra-fields-capture-hardening (deferred, pre-existing, LOW severity). Normalizer warning/error text quotes user-controlled keys with manual single-quoting (e.g. capture_unknown_fields format!("'{k}'")) and via yaml_display. A key with quotes/newlines/control chars could corrupt logs or forge diagnostic lines in the section-10 error/JSONL output. Pre-existing; applies to yaml_display repo-wide; low severity for a local CLI reading a repo-local OSS-RELEASE.md. Fix: JSON-encode keys embedded in diagnostics. Related: @extra-fields-capture-hardening.
