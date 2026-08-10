---
created: 2026-08-10
updated: 2026-08-10
type: feature
status: open
priority: normal
---

# Per-distribution release + dist generate for a monorepo

## Description

_Source: /llm-review of distribution-monorepo-vec (2026-08-10)._

`distribution-monorepo-vec` landed `distributions: Vec<Distribution>` with per-package association,
but the release ENGINE and `dist generate` still only handle ONE distribution per run. Both paths now
fail LOUD on a multi-distribution monorepo (`multiple_distributions` error) rather than silently
mis-cutting. This issue tracks making them multi-distribution-aware.

## Scope
- **Release engine per-distribution taps.** The sealed `ReleasePlan` carries a single `homebrew_tap`.
  `release plan`/`cut`/`resume` reject `distributions.len() > 1` via `ensure_single_distribution`
  (crates/ossctl-cli/src/release.rs). Carry per-distribution release data (a `PlannedDistribution`
  with package + tap + installers/platforms) so each binary's formula goes to its own tap; drop the
  guard when done.
- **`dist generate --package <name>`.** `ossctl dist generate` errors `multiple_distributions` on ≥2
  distributions (crates/ossctl-cli/src/dist.rs). Add a `--package` selector that scaffolds one
  distribution's `dist-workspace.toml` at a time (still a single-file emit).
- **cargo-dist per-distribution platforms.** cargo-dist's `dist-workspace.toml` has ONE workspace-level
  `targets` list; a monorepo declaring different `platforms` per distribution can't be represented
  natively. Decide: union, per-package restriction, or reject non-uniform platforms for the cargo-dist
  adapter — and document/validate it.

## Not in scope
The contract MODEL is already done (this is purely engine/tooling catch-up).
