---
created: 2026-07-25
updated: 2026-07-25
type: epic
owner: jari
status: open
priority: high
---

# Build ossctl: the /oss-* family's deterministic core

## Description

Extract the deterministic core of the OSS-release skill family into this CLI, per the founding ADRs in docs/adr/ (0001 spine, 0002 release engine, 0003 config+journal). The prose /oss-* skills become thin callers bundled with the binary; the binary is the source of truth. Build order follows ADR-0001 dependency-first: workspace → contract → facts → audit/skill → release engine → migrate oss-init → prose skills. The locked family design lives in homebase issues/oss-release-skill-family/design.md; the ADRs realize it. This epic tracks the whole extraction.
