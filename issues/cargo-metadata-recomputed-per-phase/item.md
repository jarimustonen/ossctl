---
created: 2026-08-06
updated: 2026-08-16
type: improvement
status: wontfix
priority: normal
closed: 2026-08-16
---

# cargo-metadata-recomputed-per-phase

## Description

The cargo adapter runs read-only `cargo metadata` (and now, since the cargo-interleave change, per-dep `is_published` registry probes) independently in dry_run, build, and publish. For N cargo targets a cut runs metadata up to 3N times plus repeated dep probes. Compute the workspace dependency graph once (coordinator/plan preflight) and thread the resolved classification + dep list into the adapter, keeping it per-target. Surfaced by all four /llm-review reviewers on release-cut-build-phase-dep-ordering. Correctness is fine; pure redundancy that bites large workspaces.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Performance cleanup only, not current product or release-safety work. Closing under the no-backlog policy.
