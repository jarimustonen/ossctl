---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: done
priority: normal
labels: [cli-canon, tooling]
lane: cli-canon
lane_seq: 10
commits:
- hash: 22b4d6eb87a52edf40d23a9c6ed55f60f811624a
  summary: 'feat(cli): add config inspection commands'
- hash: ec015fcea0d926edeb53f8b395aacfc69f5bcfa6
  summary: 'fix(cli): harden config provenance reporting'
closed: 2026-08-16
---

# cli-canon: §8 config path / config show --json

## Description


Filed by the `stack-cli-alignment` CLI-surface normalisation (homebase epic), phase 1.
Source: homebase `issues/cli-alignment-audit/analysis.md` (2026-08-10 audit) + live
re-verification 2026-08-16. Canon: `AGENTS-AI-FIRST-CLI.md`. This is a **fix** issue
(the audit + review only recommend); laned in `cli-canon` for a future `/stint-start`.

**Gap (§8) — no `config path` / `config show --json`.**

An agent cannot ask "where does the effective config live" or "why is this value what it
is". This is the family's single most consistent miss (7/7 tools ✗ in the audit).

**Do:** add a `config` subcommand — `config path` (print the effective config file path)
and `config show --json` (effective config values + their source/provenance). Non-mutating,
`--json` envelope like the rest of the surface.

**Current state (evidence):** `ossctl config` → unrecognized subcommand.
