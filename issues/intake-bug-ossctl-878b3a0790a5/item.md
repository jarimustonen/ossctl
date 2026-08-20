---
created: 2026-08-16
updated: 2026-08-20
type: bug
reporter: jari
status: fixed
priority: normal
closed: 2026-08-16
provenance: agent-homebase-wrapup
---

# release plan rejects --output flag though other subcommands accept it

## Description

release plan rejects --output flag though other subcommands accept it

## Observed

`ossctl release plan --version <v> --output json` hard-fails with an unknown-flag error:

    $ ossctl release plan --version 0.11.0 --output json
    {"schema_version":1,"error":{"code":"unknown_flag","message":"error: unexpected argument '--output' found  Usage: ossctl release plan --version <VERSION>"}}

Dropping `--output` works:

    $ ossctl release plan --version 0.11.0
    plan_id:    2cb22bd7...
    ...

Other ossctl subcommands (e.g. `contract show --json`) accept structured-output flags, so
`release plan` not accepting `--output` (or `--json`) is an inconsistency.

## Expected

`release plan` should accept `--output json` (or `--json`) and emit the sealed plan
(plan_id, head, version, targets, phases) as a JSON envelope on stdout, matching the
AI-first-CLI contract the rest of the family follows. A scripted release driver can't parse
the plan_id from the human-formatted output reliably.

## Impact

Low severity (the human-readable output is parseable by hand), but it breaks scripted
release automation and is an AI-first-CLI surface inconsistency. Observed 2026-08-16 while
cutting issuectl 0.11.0.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Current release plan supports --json structured output. The specific old --output json spelling is not the canonical surface, but the AI-first automation need is covered.
