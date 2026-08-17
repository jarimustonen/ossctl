---
created: 2026-08-17
updated: 2026-08-17
type: improvement
status: open
priority: normal
---

# release abandon rejects a sealed-but-never-cut plan id

## Description

`ossctl release plan` seals a plan and persists it in the plan store. If the operator
then decides not to cut it, `ossctl release abandon <plan_id>` answers `run_not_found`:
a sealed plan is not yet a run, and `abandon` only knows runs.

**Reachable here:** yes, and observed — hit while cutting glasspad 0.4.0 (the
`release-ci-publish-mode` intake), and reachable by any user who plans then changes
their mind. **Damage beyond the error message:** small but real — the plan store keeps
accumulating sealed-but-dead plans with no documented way to drop one, and the error
names the wrong thing (it reports "no such run" for an id the tool itself just issued).

**Suggested shape:** either teach `abandon` to accept a plan id and drop the stored plan
(journalling nothing, since nothing ran), or fail with a precise error that says the id is
a *plan*, not a run, and names the command that does dispose of it.

**Reopen/close condition:** close as fixed when `abandon <plan_id>` either disposes of the
stored plan or returns an error that distinguishes a plan id from an unknown run id.
Close as wontfix only if the plan store gains its own GC that makes disposal automatic.
