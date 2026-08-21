---
created: 2026-08-21
updated: 2026-08-21
type: bug
reporter: mail-triage
status: open
priority: high
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
