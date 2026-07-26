---
created: 2026-07-25
updated: 2026-07-26
type: feature
status: done
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@workspace-scaffold']
commits:
- hash: 90a0411516497be0a0e7f048790684e0581c22e8
  summary: port infer-repo-facts.py to ossctl facts
- hash: 34cb5f21050d4745fff1aa9a8d5ea0dedf3ce27b
  summary: apply llm-review findings (char-limit, is_file port, git hardening, single-quote TOML)
closed: 2026-07-26
---

# ossctl facts — port the deterministic repo-fact detector to Rust

## Description

Port homebase's infer-repo-facts.py into ossctl-core::facts behind the repo ports, exposed as 'ossctl facts' → schema-versioned JSON (ecosystems, packages, has_ci, tags, committers, inferred_maturity). Same facts feed both /oss-init (config generation) and audit, so they never disagree on maturity or the gated core. Preserve the JSON field names the oss-init skill already relies on.
