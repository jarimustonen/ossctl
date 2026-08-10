---
created: 2026-08-06
updated: 2026-08-10
type: bug
status: fixed
priority: normal
epic: ossctl-phase4-build
commits:
- hash: c8cec19
  summary: 'fix(cli): release abandon --reason accepts leading-dash values'
closed: 2026-08-10
---

# release abandon --reason fails when the reason starts with --

## Description

_Source: stint #12, abandoning a failed 0.2.0 cut run._

## Problem
`ossctl release abandon <run-id> --reason "<text>"` fails when the reason string starts with `--`:
clap parses the value as a flag. Observed:
```
ossctl release abandon 01KZB40Y… --reason "--no-verify insufficient; cargo package still resolves …"
=> {"error":{"code":"unknown_flag","message":"error: unexpected argument '--no-verify insufficient…' found ... Usage: ossctl release abandon <RUN_ID>"}}
```
Reword-without-leading-dashes works, but a legitimate reason can begin with `--` (e.g. quoting a flag).

## Expected
Accept any reason value, including one starting with `--` — support `--reason=<value>` binding and/or
document/allow a `--` end-of-options separator so the value is taken literally. (AI-first CLI: an
informative error that names the fix would also help.)

## Impact
Minor UX papercut on a recovery command; a leading-dash reason is silently rejected.
