---
created: 2026-07-25
updated: 2026-07-26
type: feature
status: done
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@contract-command', '@facts-command']
commits:
- hash: 845e19f
  summary: implement ossctl audit readiness engine
- hash: 5fd97c0
  summary: apply llm-review findings
closed: 2026-07-26
---

# ossctl audit — readiness scoring (the /oss-readiness engine)

## Description

Score a repo vs the gated core (README+LICENSE+CI) plus the tier-scaled canon and GitHub community-standards; emit a gap-report JSON (read-only, no repo writes). Reads 'contract show' + 'facts'. Producer-existence gaps (missing fragment dir, coverage step, scorecard action, CI, LICENSE) are the AUDIT's to report — the normalizer does NOT hard-fail on them (the advisory-producer decision from the oss-init unit). Registry/GH-API lookup failure ⇒ 'unknown', never 'false'. Read-only 'gh api .../community/profile' is approved. This is the engine the /oss-readiness skill wraps.
