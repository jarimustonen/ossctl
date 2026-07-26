---
created: 2026-07-25
updated: 2026-07-26
type: feature
status: done
priority: high
epic: ossctl-phase4-build
blocked_by: ['@workspace-scaffold']
commits:
- hash: ee39196
  summary: port OSS-RELEASE.md normalizer to contract show|validate (salvaged)
- hash: 6c8362a
  summary: fix path_inside_repo ../ escape under relative repo root (llm-review)
closed: 2026-07-26
---

# ossctl contract show|validate — port the OSS-RELEASE.md normalizer/validator to Rust

## Description

THE inter-skill contract. Port homebase's check-oss-release.py (dotfiles/src/.claude/skills/oss-init/scripts/) into ossctl-core::contract as 'ossctl contract show' (canonical, defaulted, targets-expanded, schema-versioned JSON — the single reader every member calls) and 'ossctl contract validate' (pass/fail gate, §10 error envelope, no body). PRESERVE the canonical-JSON shape exactly (SCHEMA.md §4 in the oss-init skill is the contract; the subcommand names change, the JSON does not). Enforce every cross-field floor (auto-on-spike, registry⇒SPDX license, slsa-l3⇒production, badge⇒producer, schema_version bound, status:draft refused by mutators via --require-approved). Vendored SPDX check. Serde types in contract/schema.rs are the ONE canonical model (ADR-0003 §1); wire DTOs live in protocol/ (ADR-0001 §2).
