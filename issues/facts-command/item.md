---
created: 2026-07-25
updated: 2026-07-25
type: feature
status: open
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@workspace-scaffold']
---

# ossctl facts — port the deterministic repo-fact detector to Rust

## Description

Port homebase's infer-repo-facts.py into ossctl-core::facts behind the repo ports, exposed as 'ossctl facts' → schema-versioned JSON (ecosystems, packages, has_ci, tags, committers, inferred_maturity). Same facts feed both /oss-init (config generation) and audit, so they never disagree on maturity or the gated core. Preserve the JSON field names the oss-init skill already relies on.
