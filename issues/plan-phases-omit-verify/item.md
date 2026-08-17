---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: normal
lane: release-safety
lane_seq: 50
---

# sealed plan's phases list omits the verify phase the coordinator always runs

## Description

Found during the 0.7.0 cut (2026-08-17). 'ossctl release plan --json' returns phases: [bump, dry-run-all, build-all, publish-all, tag, dist] — but since journal v5 the coordinator ALWAYS runs a seventh, mandatory verify barrier after dist (a run is Completed only when every target is observed). An agent or operator inspecting the sealed plan to know what a cut will do gets an incomplete phase list; the plan is the approval artifact, so it should describe the real phase sequence. Fix: include 'verify' in the plan's phase list (additive JSON change; check whether the phase list participates in the seal pre-image — if it does, this is a SEAL_VERSION-bump change and must be handled per plan.rs's documented evolution rule, not silently).
