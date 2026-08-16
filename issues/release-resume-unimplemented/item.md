---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: obsolete
priority: high
closed: 2026-08-16
---

# release resume/verify cannot finish a partially-published run (0.2.2)

## Description

## Symptom

After a publish phase partially completed (target A published, target B failed), the run is left
`in_progress` and cannot be driven to completion:
- `release resume <run>` → `resume_conflict`: 'this run recorded a publish the registry no
  longer reports … a human must decide; ossctl will not blindly re-publish' (the recorded
  version and the registry's actual version differ — see the sibling bug release-cut-ignores-version).
- `release verify <run>` reports the recorded target as `missing`.
- The cut's own error says recovery via resume/verify 'lands in a later version; until then
  inspect the journal and reconcile the registries manually'.

## Impact

Any partial publish (a failed second crate, a propagation gap, a transient registry error) strands
the release: the engine can neither resume nor cleanly re-cut (a fresh cut re-attempts the
already-published target and fails 'already uploaded'). Operators must finish entirely by hand
(manual cargo publish + tag + formula), defeating the resumable-engine design.

## Expected

`release resume` should skip already-satisfied targets (idempotent by receipt / registry state)
and continue with the remaining phases (publish-remaining → tag → dist). `--allow-unverified`
exists but did not cover this path. Note this is partly downstream of release-cut-ignores-version:
fixing the version handling removes the 'recorded 0.1.0 but registry has 0.0.0' conflict class.

## Evidence

project-canon 0.1.x release, 2026-08-16. Two runs abandoned; the release was finished manually.

## Env
ossctl 0.2.2.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Obsolete 0.2.2-era report, partly downstream of the removed --version drift. Refile a current, narrow resume bug if 0.5.x still strands a partially published run.
