---
created: 2026-09-06
updated: 2026-09-06
type: bug
status: open
priority: high
lane: release
lane_seq: 1
collision: [crates/shipshape-core/src/release/delegated.rs]
---

# Observe cargo publish workflows through tracked helpers

## Summary

Shipshape cannot reconcile a successful `cargo-publish-ci` target when a tag-triggered GitHub Actions workflow delegates each publication to a checked-in repository-local helper such as `./scripts/publish-crates.sh publish <package>`. The observer only runs `git grep "cargo publish" v<version> -- .github/workflows/*`, so it reports every crate target as `Unknown` even when the exact workflow run and all ordered publish jobs succeeded.

## Observed occurrence

Taskfleet v0.6.1 published all three crates successfully in GitHub Actions run `34020495272`, but Shipshape journal `01M1TTRXNXK6FPQJK3F92B9AXA` remains in progress. Both Shipshape 0.10.1 and 0.12.0 return: `no tracked GitHub Actions workflow containing cargo publish could be resolved for the cargo-publish-ci target`. GitHub Release and Homebrew reconcile as `Matches`.

The tagged workflow invokes `./scripts/publish-crates.sh publish taskfleet-core|taskfleet|orchestratectl`; that tracked helper contains and controls the real `cargo publish` operations and receipt generation.

## Required outcome

- Resolve a unique tag-triggered workflow that either contains direct `cargo publish` or invokes a tracked repository-local helper that contains the publish operation.
- Pin all inspection to the immutable release tag tree; do not follow mutable main or arbitrary external actions.
- Fail closed on absent, ambiguous, untracked, escaping, recursive, or otherwise unverifiable helper references.
- Preserve exact-tag workflow-run and job-outcome verification.
- Add regression tests using the Taskfleet-shaped helper topology and adversarial ambiguous/missing helper cases.
- Keep `Unknown` red when ownership cannot be proven.
- Land through the full Shipshape green gate. Do not publish a Shipshape release as part of this worker.
