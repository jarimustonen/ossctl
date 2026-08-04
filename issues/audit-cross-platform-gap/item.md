---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@distribution-cross-platform-targets']
---

# audit should flag a distribution missing Linux targets as a readiness gap

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

Enforce the 'all OSS software installs on macOS AND Linux' policy in the readiness engine. `ossctl audit` (crates/ossctl-core/src/audit/) should flag a `distribution` block whose target set omits Linux (no *-unknown-linux-* triple) as a readiness gap — recommended at mvp, stronger at production. Makes the cross-platform requirement self-checking rather than convention-only. Depends on distribution-cross-platform-targets (needs the target field to inspect).
