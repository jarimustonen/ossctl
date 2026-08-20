---
created: 2026-08-18
updated: 2026-08-20
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent-homebase-wrapup
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
