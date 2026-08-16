---
created: 2026-08-04
updated: 2026-08-16
type: task
status: duplicate
priority: normal
epic: ossctl-phase4-build
related: ['@homebrew-publish-resume-idempotency']
closed: 2026-08-16
---

# Journal homebrew create sub-steps / reconcile remote for safe resume

## Description

The create path (clone -> write -> branch -> commit -> push -> PR) is per-target irreversible but only the final receipt is journaled. A crash/interrupt after push or PR leaves remote state; resume re-runs the whole sequence and can fail (remote branch/PR already exists). Either journal sub-steps like tag_phase, or reconcile remote state (detect existing branch/open PR and adopt it). Raised by /llm-review of homebrew-adapter-first-formula (F9).

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Subsumed by the retained Homebrew resume/idempotency hardening issue.
