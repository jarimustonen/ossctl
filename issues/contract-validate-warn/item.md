---
created: 2026-08-15
updated: 2026-08-16
type: improvement
status: done
priority: normal
lane: release-safety
commits:
- hash: 2aa530a589b1712c14f101dd116d21cd250443b5
  summary: 'fix(contract): warn on unplanned Homebrew tap'
closed: 2026-08-16
---

# contract validate: warn when dist-workspace.toml declares a homebrew tap absent from the contract distribution block

## Description

**The gap.** `ossctl release plan` sources the plan's `homebrew_tap` from the contract's
`distribution:` block in `OSS-RELEASE.md` (see `release/plan.rs`), NOT from `dist-workspace.toml`.
This is correct-by-design — the contract is the single source of truth (ADR-0003). But the failure
mode is SILENT: a downstream repo whose Homebrew tap is declared only in `dist-workspace.toml`
(cargo-dist config) and NOT mirrored into a contract `distribution:` block gets `homebrew_tap: null`
in the plan — so a `release cut` would quietly **drop the Homebrew leg** with no error and no warning.

**How it surfaced (stint #20, 2026-08-14).** Preparing orchestratectl for its `--bump` cut, the
first `ossctl release plan --bump minor` carried a null tap even though `dist-workspace.toml` had a
valid `publish-jobs = ["homebrew"]` + tap. The fix there was to add a `distribution:` block to
orchestratectl's `OSS-RELEASE.md` — but nothing told us the tap was missing; it was caught only by
eyeballing the plan output. A less careful downstream cut would have shipped crates.io + GH-release
and silently skipped Homebrew.

**Proposed improvement.** In `ossctl contract validate` (and/or `release plan`), when a
`dist-workspace.toml` is present and declares a Homebrew tap / `publish-jobs = ["homebrew"]` but the
contract's `distribution:` block omits `homebrew_tap` (or there is no `distribution:` block at all),
emit a **WARNING**: the tap in dist-workspace.toml will NOT be planned; declare it in the contract
`distribution:` block. Keep it a warning (not an error) — the contract stays authoritative; this only
makes the drift visible. Consider the symmetric case (contract declares a tap dist-workspace.toml
doesn't) as a lower-priority note.

**Scope / files.** `crates/ossctl-core/src/contract/*` (validate path) or the plan-time diagnostics;
read `dist-workspace.toml` for the cross-check (a new read — currently the contract path doesn't parse
it). Non-blocking, additive; no schema change. Fail-safe: if `dist-workspace.toml` is absent or
unparseable, no warning.

## Done

`ossctl contract validate` (or `release plan`) emits a clear warning when a Homebrew tap declared in
`dist-workspace.toml` is not reflected in the contract's `distribution.homebrew_tap`, so a downstream
repo can't silently plan a cut that drops its Homebrew leg. Fail-safe when dist-workspace.toml is
absent/unparseable.

