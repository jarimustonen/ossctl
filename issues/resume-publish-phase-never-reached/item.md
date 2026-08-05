---
created: 2026-08-05
updated: 2026-08-05
type: bug
status: open
priority: normal
epic: ossctl-phase4-build
---

# release resume demands --allow-unverified even when the publish phase was never reached

## Description

During an orchestratectl 0.1.0 release cut, `ossctl release resume <run>` returned `resume_conflict` demanding `--allow-unverified` even though the run failed in the BUILD phase, before publish-all ever started — so nothing could have been published. Root cause: in `crates/ossctl-core/src/release/resume.rs`, a not-recorded target whose ecosystem has no wired registry query (e.g. rust/cargo) verifies to `Unknown`, and the state table maps `(NotRecorded, Unknown)` → `Unverifiable` (a hard blocker) unless `--allow-unverified` is passed. This is correct WHEN a publish could have landed without a receipt (crash mid publish-all), but needlessly conservative when the publish phase was provably never entered.

Proposed fix (own change, own review — touches the safety-critical reconcile invariant 'never blind re-publish'): add a 'publish-phase-reached' signal to `reconcile_for_resume` (derive from `RunState`: current_phase >= Publish OR any completed phase >= Publish) and thread it into `classify`, so `(NotRecorded, Unknown)` resolves to `ResumePublish` instead of `Unverifiable` when the publish phase was never reached (nothing could have published). Must NOT relax the `Published × Unknown` row, nor the mid-publish crash case (publish reached but no receipt). Requires updating `classify`'s signature + the exhaustive `classify_covers_every_cell` test and the `state_with` helper in resume/tests.rs to set phases, plus the ADR-0003 §4 state-table doc. Deferred out of the list/abandon spinoff to keep that change focused and avoid risk to the reconcile safety table.

Discovered while implementing release-list-abandon-not-implemented (see that issue).
