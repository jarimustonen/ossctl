---
created: 2026-08-06
updated: 2026-08-07
type: bug
status: in-progress
priority: normal
epic: ossctl-phase4-build
commits:
- hash: 0e0dea7
  summary: fix homebrew_tap dead-config false positive for homebrew-tap targets
---

# contract validate false-positive: homebrew_tap 'dead config' warning ignores a homebrew-tap TARGET as a consumer

## Description

_Source: stint #12, sealing the 0.2.0 engine plan for ossctl's own contract._

## Problem
`ossctl contract validate` / `release plan` emit a FALSE-POSITIVE warning for ossctl's own contract:
`distribution.homebrew_tap is set but 'homebrew' is not in distribution.installers — no formula is
generated, so the tap will never be updated` (normalize.rs ~line 760).

But that is WRONG for the target-based homebrew model. ossctl declares homebrew TWO ways:
- a `targets:` entry `{rust, ossctl, homebrew, homebrew-tap}` — consumed by the release ENGINE
  (the plan's homebrew target; the coordinator's `dist` phase DOES generate + push the formula to
  the tap), and
- `distribution.installers` — consumed by cargo-dist (`dist generate` → release.yml).

`distribution.homebrew_tap` is (correctly) needed by the homebrew-tap adapter TARGET, but the
validator only knows the cargo-dist-installer path, so it asserts "the tap will never be updated"
even though the engine's dist phase updates it. Setting `homebrew` in `installers` would instead make
cargo-dist ALSO publish homebrew — a double-publish collision — so that's not the fix.

## Expected
The dead-config warning should account for a `homebrew`/`homebrew-tap` TARGET as another consumer of
`distribution.homebrew_tap` — i.e. no warning when a homebrew target is declared (the tap IS used).
Only warn when `homebrew_tap` is set AND there is neither a `homebrew` installer NOR a homebrew target.

## Impact
Cosmetic (non-blocking) — the 0.2.0 plan sealed fine with the warning. But it misleads the operator
into thinking the tap won't update, on exactly the (correct) config the engine cut needs.
