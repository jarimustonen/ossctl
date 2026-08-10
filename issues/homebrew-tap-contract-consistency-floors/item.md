---
created: 2026-08-07
updated: 2026-08-07
type: improvement
status: open
priority: normal
---

# Homebrew contract-consistency floors: target-without-tap, double-publish, registry/adapter compat

## Description

# Homebrew contract-consistency floors: target-without-tap, double-publish, registry/adapter compat

## Description

_Source: `/llm-review` (Gemini 3.1 Pro, GPT-5.6-sol, Claude Opus 4.7, DeepSeek v4 Pro),
triaged via `/assess-findings`, during the fix for `contract-homebrew-tap-warning-false-positive`
(commit 0e0dea7)._

While fixing the false-positive dead-tap warning (a `homebrew`-registry TARGET now counts as a
consumer of `distribution.homebrew_tap`), all four review models independently flagged a set of
**pre-existing, orthogonal** homebrew cross-field validation gaps that the false-positive fix
deliberately did NOT expand into. This issue collects them.

## Gaps (verified against `crates/ossctl-core/src/contract/normalize.rs`)

### 1. Inverse gap — homebrew target with no `homebrew_tap` is silent (CONSENSUS 4/4, top finding)
A contract with `{ecosystem: rust, registry: homebrew, adapter: homebrew-tap}` but **no**
`distribution.homebrew_tap` (or no `distribution` block at all) normalizes clean. The engine's
`dist` phase then has no tap destination to push the generated formula to → runtime failure.
This is symmetric to the existing installer-without-tap floor (`normalize.rs:750`,
`wants_homebrew && homebrew_tap.is_none()`), which has **no** target-side counterpart. When there
is no `distribution` block at all, `parse_distribution(None, …)` returns early and `homebrew_tap`
cannot even be expressed — so consider whether a homebrew target is even legal without a
distribution block, or whether the tap belongs at a level a registry-only contract can set.

**Decision needed (why this is its own design, not a mechanical fix):** floor (hard error) vs
advisory (warning)? The installer path is a floor; consistency argues for a floor here too, but
that is a behavior change that could reject currently-accepted contracts — needs a deliberate call.

### 2. Double-publish collision not floored (2/4)
`distribution.installers: [homebrew]` AND a `homebrew`-registry target both generate + push a
formula (cargo-dist AND the engine) — the exact collision the fix's code comment warns against.
The normalizer accepts it silently. Candidate: floor `wants_homebrew && has_homebrew_target`.

### 3. registry/adapter compatibility not enforced (2/4)
`validate_targets` (`normalize.rs:615`) accepts any adapter for `registry: homebrew` (e.g.
`adapter: manual`). The false-positive fix's `has_homebrew_target` predicate is registry-only, so a
`{registry: homebrew, adapter: manual}` target also suppresses the advisory. Debatable whether a
manual homebrew target is a "producer"; if `homebrew` registry requires the `homebrew-tap` adapter,
enforce it in `validate_targets` and narrow the predicate to `adapter == HomebrewTap`.

## Why deferred (not fixed in 0e0dea7)
The false-positive fix is a warning-suppression change; these are new floors / a tightened predicate
that (a) change accept/reject behavior and (b) need a floor-vs-warning design decision. Bundling them
would have expanded scope past the one-line false-positive fix. Filed as follow-up hardening.

## Suggested design
Introduce one cross-field `check_homebrew_configuration(&targets, distribution.as_ref(), p)` that
owns the full truth table (tap × installer-producer × target-producer): missing-tap floor for either
producer, double-publish floor when both, dead-tap advisory when neither. Add the truth-table tests
(GPT-5.6-sol's §10 matrix) covering all eight rows.

## Decision (Jari, 2026-08-10) — FLOOR (hard error)

**Chosen: hard error (floor), not advisory.** Rationale: the AI-first CLI contract requires reacting
to errors strictly and immediately — a misconfiguration must fail fast and loud, not slip through as a
warning to fail later at release time. Implement all three floors (missing-tap for either producer,
double-publish when both, registry/adapter compatibility) as hard errors, narrowing the predicate to
`adapter == HomebrewTap` where relevant. Accept that this rejects some currently-accepted contracts —
that is the intended behavior change. NOTE: read the CURRENT contract shape — `distribution-monorepo-vec`
(distributions Vec, schema_version 2) and `publish-target-none` (Option<Vec<Target>>) just landed.
