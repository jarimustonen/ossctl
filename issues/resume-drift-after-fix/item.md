---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: in-progress
priority: high
lane: release-safety
lane_seq: 3
commits:
- hash: c990144
  summary: 'feat(release): persist sealed plans for cut and resume'
---

# release resume is unusable after a code fix: the fix itself drifts the sealed plan

## Description

Hit during the 0.6.0 cut (2026-08-17).

Sequence: the cut failed at the dist phase because of a bug in the engine. The bug was fixed. `release resume <run>` then refused:

    run ... was sealed against plan e94fcec8566d, but the current repository (HEAD 3a939677ccaf,
    version 0.6.0) hashes to a different plan_id — a commit, a contract or manifest edit, a version
    change, or an uncommitted working-tree change occurred since the cut

The guard is correct in itself: continuing a different plan under a sealed run id would defeat the whole content-addressed seal. But note what it means in practice.

**The most common reason a cut fails partway is a defect in the engine or the contract. Fixing that defect necessarily moves HEAD. Therefore the fix necessarily makes resume refuse.** So `resume` can only ever recover from TRANSIENT failures — a network blip, an expired token, a CI timeout. It cannot recover from the failure class that actually stopped this cut, and that has stopped several before it (0.2.0 build, 0.2.1 publish, 0.2.2 dist, 0.6.0 dist — every one of them a code defect).

The recovery that remains is: abandon the run, cut a new version. That works, but it burns a version number for a defect that never reached a user, and it leaves the earlier version permanently half-published — 0.6.0 is on crates.io and GitHub with no Homebrew formula, forever.

## Worth considering

- A resume that re-seals: recompute the plan from the current tree, verify the ALREADY-PUBLISHED targets still match their receipts (the digest/reconcile machinery for this exists), and continue only the outstanding phases under a new plan id linked to the old run. The integrity property that matters is 'what was published matches what was recorded', not 'the tree never changed'.
- Or an explicit, narrow escape: continue only the phases that produce no registry effect (dist/tap), which is exactly the case here.
- Or accept the limitation and make it explicit in the error: say that a code fix invalidates resume by construction and that abandon-and-recut is the intended path. Even this would be an improvement — the current message reads as if the user did something careless.

Whichever direction, decide it deliberately; today the limitation is undocumented and discovered at the worst moment.

## Acceptance
- The behaviour after an engine-fix-then-resume is a recorded decision, not an accident.
- If resume stays strict, its error names abandon-and-recut as the intended recovery.
- If resume gains a re-seal path, published targets are digest-verified before any outstanding phase runs.

## Comments

### 2026-08-17T06:21:46Z · @pi

Shipped durable sealed-plan storage. release resume loads the sealed stored plan and no longer re-derives from live HEAD when the plan is available, so it can resume after a code fix.
