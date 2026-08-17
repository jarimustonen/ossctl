---
created: 2026-08-17
updated: 2026-08-17
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
lane: cli-canon
lane_seq: 50
---

# ossctl --version is rejected as an unknown flag; it should alias the ve…

## Description

ossctl --version is rejected as an unknown flag; it should alias the version verb

`ossctl --version` fails with an unknown-flag error instead of printing the
version. The information is available only through the `version` verb, so the
single most reflexive thing anyone (human or agent) types to identify a binary
does not work.

## Observed

    $ ossctl --version
    {"schema_version":1,"error":{"code":"unknown_flag","message":"error: unexpected argument '--version' found  Usage: ossctl [OPTIONS] <COMMAND>"}}

    $ ossctl version
    ossctl 0.6.1
    commit:            2846d661...
    schema version:    1
    supported schemas: 1
    bundled skills:    10

Hit while verifying a fleet-wide upgrade from 0.2.2 to 0.6.1: the first probe of
every machine was `ossctl --version`, and every one of them errored before
falling back to the verb.

## Expected

`--version` (and conventionally `-V`) prints the same version information as the
`version` verb, exit 0. project-canon has the same defect filed as
`version-flag-alias` ("--version must equal the version verb"), so this is a
family-wide convention worth fixing consistently.
