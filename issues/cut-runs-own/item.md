---
created: 2026-08-17
updated: 2026-08-21
type: bug
status: in-progress
priority: high
collision: [crates/ossctl-cli/src/release.rs, crates/ossctl-core/src/release/distribution.rs]
lane: verify-seam
lane_seq: 20
provenance: agent-issuectl-stint
commits:
- hash: ef39fca
  summary: fix release double-writer routing, dist retries, and post-failure verification
- hash: 1d3aa66
  summary: record initial fix commit on issue
- hash: e02c840
  summary: apply review findings for safe retry and observation-only verify
---

# cut runs its own homebrew leg despite cargo-dist publish-jobs owning the tap: double-writer, no retry on 503, false-red failed run

## Description

## Summary

On a contract whose `distribution.adapter` is `cargo-dist` AND whose cargo-dist config
(`dist-workspace.toml`) itself carries `publish-jobs = ["homebrew"]`, `ossctl release cut`
(0.7.0) still executes its **own** homebrew leg in the dist phase (clone tap → render → push).
That leg is (a) a **double-writer** against cargo-dist's `publish-homebrew-formula` job for the
same formula, and (b) a **single point of failure that fails the whole cut** even when every
target is actually delivered.

Observed on issuectl's 0.15.0 cut (run `01M08CFSBMDSKEQV9FPR7FXF7C`, ossctl 0.7.0):

- dry-run/build/publish/tag all green; crates.io receipts recorded for both crates; tag pushed;
  GitHub Release correctly **delegated** to cargo-dist (the 0.6.1 pre-create bug is fixed — confirmed).
- dist phase: `gh repo clone jarimustonen/homebrew-issuectl … --depth 1` got a transient
  GitHub `HTTP 503` → dist phase `failed` → **exit 2, run recorded as failed, and the verify
  barrier never ran**.
- Meanwhile cargo-dist's tag-triggered workflow completed: GitHub Release with 12 assets AND
  the tap formula advanced to 0.15.0. So the release was fully delivered while ossctl reported
  a failed cut — the inverse of the 0.6.1 false-green, a false-red.

## Expected

1. When cargo-dist's own config declares `publish-jobs = ["homebrew"]`, the homebrew target
   should be **delegated** (observed/verified, like gh-releases) rather than served by ossctl's
   own renderer — one writer per destination. At minimum, detect the double-writer and warn.
2. A transient network failure in one leg should retry (503 is retryable) instead of
   terminally failing the phase on the first attempt.
3. When the dist phase fails after irreversible publishes, the verify phase should still run
   (or be runnable) so the operator learns what actually landed — the error text says
   `release verify <run-id>` / `release resume <run-id>` "lands in a later version", which
   leaves no engine-side recovery for exactly the case the journal was built for.

## Impact

Operator must manually reconcile registries (crates.io API, `gh release view`, tap formula) to
discover the release actually succeeded; the run journal permanently records a failed cut for a
delivered release.
