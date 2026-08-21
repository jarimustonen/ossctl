---
created: 2026-07-25
updated: 2026-08-21
type: epic
owner: jari
status: done
priority: high
closed: 2026-08-21
closed_by: agent-stint-24
---

# Build ossctl: the /oss-* family's deterministic core

## Description

Extract the deterministic core of the OSS-release skill family into this CLI, per the founding ADRs in docs/adr/ (0001 spine, 0002 release engine, 0003 config+journal). The prose /oss-* skills become thin callers bundled with the binary; the binary is the source of truth. Build order follows ADR-0001 dependency-first: workspace → contract → facts → audit/skill → release engine → migrate oss-init → prose skills. The locked family design lives in homebase issues/oss-release-skill-family/design.md; the ADRs realize it. This epic tracks the whole extraction.

## Resolution

### 2026-08-21T07:31:51Z · @agent-stint-24

Closed as delivered (maintainer decision, stint #24, 2026-08-20).

Every stage of this epic's own build order is shipped: workspace, contract, facts, audit/skill, release engine, migrated oss-init, and the prose skills. Concretely — the two-crate workspace exists; `contract show/validate`, `facts`, `audit`, `dist`, `doctor` and the full `release` verb set are live; 10 /oss-* skills ship bundled in the binary's CATALOG; and the release engine has cut 10 versions through 0.10.0, six of them engine-driven end to end with the verify barrier green.

It also tracked nothing mechanically: zero issues ever carried `epic: ossctl-phase4-build`, and the body was never updated after its creation on 2026-07-25. It was a narrative container for an extraction that is now complete.

Deliberately NOT recycled into a 1.0 tracker. Its scope had already started drifting — it was written to close when the release-safety and cli-canon lanes drained, both of which drained and refilled with unrelated work. An epic whose scope silently changes can never be audited: you can no longer say what it promised or when it was met. The 1.0 gate needs its own epic with a checkable condition (all four fleet shapes proven in real cuts — currently 1/4; a ~2-week soak with no new HIGH findings; and a written stability contract naming which JSON shapes, exit codes, and store formats freeze). Recorded in TODO.md's handoff rather than left implicit here.

Reopen condition: reopen only if a stage listed in the build order above turns out to be materially unimplemented — not for new release-engine work, which belongs to its own lane or to the 1.0 epic.
