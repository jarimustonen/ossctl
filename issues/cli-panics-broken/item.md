---
created: 2026-08-17
updated: 2026-08-17
type: bug
reporter: jari
status: in-progress
priority: normal
lane: cli-canon
lane_seq: 20
commits:
- hash: 392abf28978be13ccea41b883ee81a11b3a03259
  summary: centralize fallible stdout writes and treat broken pipes as success
- hash: 67e0243d04dd7df0f23d1926f65e8eb8f4b1f71d
  summary: apply review fixes for checked flushes and compiler-enforced stdout routing
---

# CLI panics on broken pipe

## Description

Text-mode CLI output uses println!, which panics when stdout closes early (for example a downstream consumer in a pipeline). This contradicts the CLI no-panic/error-envelope guarantee. Centralize fallible stdout writing and define consistent BrokenPipe behavior across text handlers.
