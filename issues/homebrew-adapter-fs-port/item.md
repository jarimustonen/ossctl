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

# Add a filesystem-write port to EffectCtx (homebrew formula write)

## Description

The homebrew first-formula create writes the generated .rb via std::fs directly, the one exception to the 'adapters touch no real fs' contract (documented in adapters/mod.rs EffectCtx). Add a minimal injected FileWriter/Workspace port so the write goes through a fake in tests and the seam stays honest. Raised by the 4-model /llm-review of homebrew-adapter-first-formula (F4).

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Folded into the retained Homebrew adapter hardening cluster rather than tracking as a standalone backlog item.
