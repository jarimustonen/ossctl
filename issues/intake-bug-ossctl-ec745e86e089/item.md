---
created: 2026-08-18
updated: 2026-08-20
type: bug
reporter: jari
status: fixed
priority: normal
provenance: agent-homebase-wrapup
closed: 2026-08-20
closed_by: agent-stint-23
---

# ossctl repo schema predates intake statuses: issuectl intake file fails…

## Description

ossctl repo schema predates intake statuses: issuectl intake file fails with schema-violation

Observed (2026-08-17): filing a bug into the ossctl repo with the standard flow failed:

    $ issuectl --root ~/Sources/ossctl intake file --type bug --title ... --provenance ... --body-file ...
    {"error":{"code":"schema-violation","message":"schema: field \"status\" = \"untriaged\" is not
    in allowed set [open, in-progress, testing, done, fixed, wontfix, duplicate, cannot-reproduce, obsolete]"}}

ossctl's issues/.schema.yaml predates issuectl's first-class intake statuses (untriaged etc.),
so `intake file` — the documented standard reception path — cannot be used in the ossctl repo.
The workaround used was the legacy shape (`issuectl create ... --label needs-triage --label
via:agent-...`), which the intake transitions in turn do not accept.

(Filed as type:bug because intakectl only accepts bug|feature; treat as a config chore.)

Expected: refresh ossctl's issue schema to admit the intake lifecycle statuses (issuectl
`intake migrate` / schema update), so ossctl receives bug reports through the same standard
intake flow as the other repos.

## Resolution

### 2026-08-20T05:47:50Z · @agent-stint-23

Fixed across the whole active fleet, not just ossctl (2026-08-20).

SCOPE. The report named ossctl, but the same schema lag affected seven of eight active repos. Only project-canon and intakectl already carried the intake statuses. Fixed in: ossctl, issuectl, orchestratectl, glasspad, homebase (five parallel workers), then deutschpad and aggountant (found when deutschpad's migration failed 13 items on the same schema-violation this issue reports).

WHAT SHIPPED. 1) issues/.schema.yaml status enum extended with untriaged / deferred / needs-info, mirroring project-canon's reference schema. 2) issuectl intake migrate --apply run everywhere, converting label-encoded intake state to first-class fields: via:* labels became the provenance field, needs-triage + open became status untriaged, label-deferred became status deferred; stale deferred labels on already-closed items were dropped without reopening. 3) issuectl doctor --fix applied.

VERIFIED. issuectl intake file now succeeds against these repos (it produced the schema-violation quoted in this report before). Zero legacy needs-triage labels remain in any of the eight; 135 issues now carry structured provenance. issuectl --json ls validates everywhere.

SIDE EFFECT WORTH KNOWING. The migration surfaced 12 untriaged bug reports in deutschpad that had been invisible to every status query because their state lived in a label. They are now reachable via issuectl intake queue.

REOPEN CONDITION. Reopen if issuectl intake file fails with a schema-violation against any active repo — most likely cause would be a NEW repo bootstrapped from an old schema template, or issuectl adding a further lifecycle status the enums do not yet carry. Note issuectl init's scaffold was not audited here: if new repos keep arriving with the old enum, that scaffold is the real fix and belongs in issuectl.
