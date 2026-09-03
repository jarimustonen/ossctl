---
created: 2026-09-03
updated: 2026-09-03
type: bug
status: in-progress
priority: high
provenance: other
provenance_detail: Observed release-cut failure assigned through orchestratectl
source_ref: orchestratectl:01m1kkravtfm7pe67bsxc9m9pj/task:bump-pin-discovery
originating_run: 01m1kkravtfm7pe67bsxc9m9pj
originating_run_kind: spinoff
---

# Bump planner misses pins in non-published workspace members

## Description

## Description

The release bump planner can omit exact intra-workspace dependency pins declared by non-published workspace members. On commit `ebce5c6`, the 0.12.0 minor plan left the non-published cargo-dist wrapper's `shipshape-cli = "=0.11.0"` requirement unchanged while staging the workspace packages at 0.12.0. The bump phase then failed during `Cargo.lock` refresh because Cargo could not resolve that stale exact pin.

The planner must discover and seal rewrites for every safe exact intra-workspace pin across all workspace members, regardless of whether the depending package is itself a publish target. Preserve package aliases and exact-pin-only behavior.

## Reproduction

1. Use the clean source tree at `ebce5c6` with the non-published `crates/shipshape-dist` wrapper.
2. Build the release binary and seal a minor bump plan from 0.11.0 to 0.12.0.
3. Execute the bump edit set and refresh `Cargo.lock`.
4. Observe that the wrapper still requires `shipshape-cli = "=0.11.0"`, so lockfile refresh fails against the staged 0.12.0 package.

## Acceptance criteria

- Workspace-wide discovery includes exact pins from non-published members.
- A regression fixture with a non-published wrapper exactly pinning a published CLI proves staged `Cargo.lock` refresh succeeds.
- Seal-version implications are evaluated under the documented evolution rule.
- The full repository green gate passes.
