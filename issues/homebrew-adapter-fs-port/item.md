---
created: 2026-08-04
updated: 2026-08-04
type: task
status: open
priority: normal
epic: ossctl-phase4-build
---

# Add a filesystem-write port to EffectCtx (homebrew formula write)

## Description

The homebrew first-formula create writes the generated .rb via std::fs directly, the one exception to the 'adapters touch no real fs' contract (documented in adapters/mod.rs EffectCtx). Add a minimal injected FileWriter/Workspace port so the write goes through a fake in tests and the seam stays honest. Raised by the 4-model /llm-review of homebrew-adapter-first-formula (F4).
