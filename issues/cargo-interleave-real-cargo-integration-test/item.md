---
created: 2026-08-06
updated: 2026-08-16
type: task
status: wontfix
priority: normal
closed: 2026-08-16
---

# cargo-interleave-real-cargo-integration-test

## Description

The cargo-interleave unit/coordinator tests use string-matching command fakes that cannot reproduce real cargo's index resolution (cargo package resolving a =-pinned workspace dep against the index even with --no-verify). Premise is established empirically by the two failed 0.2.0 engine cuts but lacks in-tree regression proof. Add a gated (#[ignore]-by-default) integration test with a real temp cargo workspace + isolated local registry demonstrating: (a) a =-pinned dependent's cargo package --no-verify fails before the dep is indexed; (b) a leaf packages; (c) after publishing+indexing the dep, the dependent packages+publishes. Surfaced by all four /llm-review reviewers.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Useful test hardening, but not current product work. Closing under the no-backlog policy.
