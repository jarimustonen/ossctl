---
created: 2026-08-21
updated: 2026-08-21
type: bug
reporter: mail-triage
status: in-progress
priority: high
lane: verify-seam
lane_seq: 5
collision: [crates/ossctl-cli/src/release.rs]
---

# CI fails in release verification and Clippy

_Source: GitHub Actions CI run 32425237330_

## Description

The `CI` workflow is red on current `main` and on the clap 4.6.6 Dependabot branch (run 32425237330). Two independent regressions fail every supported runner:

1. `release_verify_reconciles_a_journaled_run` fails on Ubuntu and macOS at `crates/ossctl-cli/tests/cli.rs:893`: expected one missing target, observed two (`left: Number(2)`, `right: 1`). The journal fixture has Python and binary targets; the changed verification behavior now classifies both as missing instead of preserving Python as unknown when its registry lookup cannot run.
2. Clippy rejects `crates/ossctl-cli/src/release.rs:208` (`result_map_or_else`): `origin_url().ok().is_some_and(...)` should use a direct `Result` combinator such as `is_ok_and(...)` (or explicit matching) rather than converting to `Option`.

Failing run: https://github.com/jarimustonen/ossctl/actions/runs/32425237330

## Root cause reading

The release-verification expectation and implementation have diverged after active registry observation was introduced, while the new source-tree check also violates the repository's deny-warnings Clippy gate.

## Concrete fix

- Decide and encode the intended Python network/lookup failure state: if an unavailable lookup is still `unknown`, restore that classification; if it is now intentionally `missing`, update the fixture assertion and surrounding contract comments consistently.
- Replace `.ok().is_some_and(...)` with direct `Result` handling accepted by Clippy.
- Run `cargo clippy --all-targets --all-features -- -D warnings` and the full cross-platform test suite.

## Comments

### 2026-08-21T07:33:53Z · @agent-stint-24

Laned verify-seam/5 (stint #24) — ahead of everything else in the lane because main is RED and 0.10.0 was cut from that red tree.

Reported by mail-triage from CI run 32425237330 (a Dependabot PR), but confirmed independently: the same two failures are present on run 32425087241, which is the 'release: v0.10.0' commit itself, and on the two commits before it. This is not a Dependabot-branch artifact.

Why the local green gate missed both, which is the more important finding:
- The test PASSES locally and fails on both CI runners, so it is environment-dependent, not a straightforward regression.
- Local clippy is 0.1.97 (rustc 1.97.1, 2026-07-14); CI runs a newer toolchain that flags result_map_or_else at release.rs:208. AGENTS.md's green gate therefore does not reproduce CI.

Severity reading for the test failure: it is not cosmetic. Classifying a Python target as 'missing' when its registry lookup cannot run contradicts ADR-0002 ('Unknown is not green' but absence of evidence is never evidence) and would fail an otherwise-healthy cut for a downstream repo with a delegated Python target. Adjacent to delegated-registry-verify-destination (verify-seam/40), which covers the routing half of the same area — check whether that issue's fix subsumes this one before implementing both.

Reopen/close condition: close as fixed when CI is green on main for both the clippy and test jobs, AND the intended unknown-vs-missing classification for an unavailable registry lookup is encoded deliberately (either the implementation restores 'unknown' or the fixture and the surrounding contract comments are updated to say 'missing' is intended). Reopen if the local green gate again diverges from CI on a released commit.
