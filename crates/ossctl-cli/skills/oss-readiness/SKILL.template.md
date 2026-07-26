---
name: oss-readiness
description: >-
  Score a repository's OSS-release readiness and turn the gap report into a
  prioritized action list. A thin skill over `ossctl audit` — the scoring
  engine lives in the binary; this skill wraps it for user-facing sequencing.
cli_version: "{{CLI_VERSION}}"
schema_version: {{SKILL_SCHEMA_VERSION}}
---

# /oss-readiness

Read-only readiness audit. The scoring engine is deterministic and lives in the
`ossctl` binary; this skill wraps `ossctl audit` for user conversation and
turns its gap report into an ordered plan.

> **Binary is the source of truth (§17).** Authored against `ossctl`
> **{{CLI_VERSION}}**. Re-run `ossctl skill print oss-readiness` if
> `ossctl version --json` reports a different `version`.

## Run the audit

```bash
ossctl contract show --json || exit    # gate: contract must normalize
ossctl audit --json || exit            # the gap report
```

## Turn gaps into actions

Each gap in the audit's report maps to a member skill (README, CI, CHANGELOG,
CONTRIBUTING, SECURITY, …). Sequence them highest-severity first, re-running
`ossctl audit --json` after each until no blocking gap remains.

> Founding mechanism template — full prose lands with the `prose-skills` unit.
