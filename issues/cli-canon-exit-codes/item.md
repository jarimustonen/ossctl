---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: open
priority: normal
labels: [cli-canon, tooling]
lane: cli-canon
lane_seq: 30
---

# cli-canon: §2 exit-code semantics (user error = 1)

## Description


Filed by the `stack-cli-alignment` CLI-surface normalisation (homebase epic), phase 1.
Source: homebase `issues/cli-alignment-audit/analysis.md` (2026-08-10 audit) + live
re-verification 2026-08-16. Canon: `AGENTS-AI-FIRST-CLI.md`. This is a **fix** issue
(the audit + review only recommend); laned in `cli-canon` for a future `/stint-start`.

**Gap (§2) — exit-code semantics diverge from the canon.**

The canon contract: **0** = success, **1** = user error (fix your input), **2** =
usage/operational error (retry/escalate). Agents branch on this. This tool does not map it
cleanly.

**Do:** audit every error exit and map user errors (not-found, bad value) → **1**, and
usage/operational (bad flag, I/O fault) → **2**. Add a test asserting the mapping.

**Current state (evidence):** `ossctl contract show` (config-not-found, a user error) exits 2, not 1.
