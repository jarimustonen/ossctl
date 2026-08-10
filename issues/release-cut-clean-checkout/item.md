---
created: 2026-08-10
updated: 2026-08-10
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@release-drop-version-flag']
---

# run release cut/resume from a clean checkout of the sealed HEAD (dirty-tree/TOCTOU)

## Description

cut/resume publish from the live mutable working tree, so the tree could change between sealing the plan and publishing — the version guard is only a point-in-time check. Execute the cut from a fresh clean checkout of plan.head_sha instead, making a cut fully reproducible and immune to mid-cut edits. LOW urgency: mostly theoretical for a solo maintainer not editing during a cut, but good 'production-grade' hygiene. Filed from the cut-noop /llm-review (stint #16).
