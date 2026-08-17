---
created: 2026-08-17
updated: 2026-08-17
type: bug
reporter: jari
status: in-progress
priority: normal
lane: cli-canon
lane_seq: 20
---

# CLI panics on broken pipe

## Description

Text-mode CLI output uses println!, which panics when stdout closes early (for example a downstream consumer in a pipeline). This contradicts the CLI no-panic/error-envelope guarantee. Centralize fallible stdout writing and define consistent BrokenPipe behavior across text handlers.
