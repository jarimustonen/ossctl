---
created: 2026-08-17
updated: 2026-08-21
type: improvement
status: open
priority: normal
lane: plan-seal
lane_seq: 20
collision: [crates/ossctl-core/src/release/distribution.rs, crates/ossctl-core/src/release/plan.rs]
---

# release cut does not check that a CI-delegated publish workflow exists

## Description

A contract can declare `adapter: cargo-publish-ci` in a repo whose tag-triggered publish
workflow does not exist, is not tag-triggered, or is `workflow_dispatch`-only. The cut
then runs its gates, pushes the tag, and waits 20 minutes for a publish nothing will
perform, ending `Missing` after the irreversible step.

**Reachable here:** yes — a repo one workflow rename away from it is the mode's
motivating case (glasspad). **Damage:** a failed post-tag run. Loud, not silent, and
recoverable (fix CI, re-run the workflow, `ossctl release verify` / `release resume`
observe the result), which is why it was not fixed alongside the mode itself: the three
refusals that DID land (already-published version, double publisher, delegated dependency
edge) each prevented a silent or unrecoverable outcome, and this one prevents a slow,
noisy, recoverable one.

**Suggested shape:** `facts` already records `tag_triggered_workflows`. At plan time,
warn — and at cut time consider refusing — when a `cargo-publish-ci` target is declared
and no `.github/workflows/*.y{,a}ml` has both a tag-push trigger and a `cargo publish`
step. Heuristic, so a warning with an override is likelier right than a hard floor;
compare the existing dead-tap advisory.

**Reopen/close condition:** close as fixed when planning a `cargo-publish-ci` contract in
a repo with no tag-triggered publish workflow surfaces a warning naming the missing
workflow. Close as wontfix if the detection proves too heuristic to be trustworthy (e.g.
it cannot see a reusable/called workflow) — record which shape defeated it.
