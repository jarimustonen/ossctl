---
created: 2026-09-06
updated: 2026-09-06
type: bug
status: fixed
priority: high
lane: release
lane_seq: 2
collision: [crates/shipshape-core/src/release/adapters/homebrew.rs, crates/shipshape-core/src/release/reconcile.rs]
closed: 2026-09-06
commits:
- hash: a7b2ae1
  summary: reconcile Homebrew GNU platforms
---

# Verify cargo-dist Homebrew GNU platform stanzas

## Summary

Shipshape's cargo-dist Homebrew reconciliation false-reds formulas built for Linux GNU targets. `homebrew_platform_condition` recognizes only `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`, while cargo-dist validly emits Homebrew formulas for the corresponding GNU targets. The expected set therefore omits Linux, and complete GNU formula stanzas are misclassified as unexpected extras (`Conflicts`).

## Observed occurrence

Taskfleet v0.6.1's sealed plan and cargo-dist manifest declare `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-gnu`. Its published formula has complete URL+sha stanzas for both and installed successfully through cargo-dist CI. Shipshape reconciliation returns `Conflicts` for `rust:taskfleet:homebrew` with the misleading detail that the Release manifest has a different tag/version, while all four other targets are `Matches`.

## Required outcome

- Treat supported Linux GNU and musl target triples as the same Homebrew OS/CPU conditions for formula verification and engine rendering/asset checks where applicable.
- Preserve exact-set behavior: missing and truly extra platform stanzas remain red.
- Reject unsupported platforms rather than silently accepting them.
- Add cargo-dist-shaped regression tests for mixed macOS ARM + Linux GNU plans, including exact expected formula stanzas and no false conflict.
- Improve delegated Homebrew non-match detail so a platform-stanza mismatch is not mislabeled as a Release-manifest tag/version mismatch if feasible without breaking the JSON schema.
- Run the full Shipshape green gate. Do not publish or globally install Shipshape.

## Resolution

### 2026-09-06T08:41:39Z · @issuectl

Linux GNU and musl targets now map to the same Homebrew OS/CPU conditions while duplicate libc variants fail closed. The full green gate passed, and repository-local Shipshape verification observed all five Taskfleet v0.6.1 targets as Matches.
