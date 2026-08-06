---
created: 2026-08-06
updated: 2026-08-06
type: bug
status: in-progress
priority: high
epic: ossctl-phase4-build
commits:
- hash: 8fc1e85
  summary: wire crates.io RegistryQuery for ecosystem rust (harvested stranded draft)
---

# release publish-phase fails closed: no crates.io RegistryQuery wired for rust

## Description

_Source: first engine cut that got PAST the build phase — the 0.2.1 dogfood, run 01KZBDST6NPYY4GHHNEXTZXWN6 (2026-08-06). The interleave fix (release-cut-build-phase-dep-ordering, commits ce85309/35f9c23) WORKS: build-all completed for all four targets incl. `rust:ossctl:crates.io` (the previously-fatal step). The cut now fails one layer deeper, in the PUBLISH phase._

## Problem

`ossctl release cut` fails at `publish-all` on the FIRST crates.io target:

```
publish-phase failed on target `rust:ossctl-core:crates.io`: cannot reach the registry to
determine the published state of `ossctl-core@0.2.1` (registry unreachable: no registry query
wired for ecosystem 'rust' yet); failing closed rather than risk an unsafe publish decision.
```

The registry-aware defer predicate + one-target-one-publish-unit publish path (ADR-0004) queries
the registry to decide whether a crate/dep is already published (idempotent resume, defer-only-for-
not-yet-published). That RegistryQuery is **not implemented for ecosystem `rust` / crates.io** — so
the publish path fails **closed** (correct, safe: it refuses to publish rather than guess).

## Impact

Blocks the engine-driven cut of ossctl itself at the publish phase. **Failure is SAFE**: it happens
BEFORE any upload (at the "determine published state" step of the first target), so nothing was
published and no tag was created; run abandoned cleanly, `published_targets: []`. This is now **THE
last blocker before the engine can dogfood its own cut** — the build-phase blocker
(release-cut-build-phase-dep-ordering) is fixed and past.

## Fix

Wire a real crates.io RegistryQuery for ecosystem `rust`: query the crates.io index/API for a
crate@version's published state (exists? checksum?) so the publish path can make its idempotent /
defer decision instead of failing closed. Likely overlaps with
`cargo-publish-receipt-provenance-resume-safety` (which already calls for a "RegistryQuery checksum").
Consider whether these two should merge. Preserve fail-closed on genuine registry-unreachable.

## Meanwhile

0.2.1 ships via the MANUAL fallback recipe (as 0.2.0 did). 0.2.1 prep (bump + CHANGELOG) is on main
at 56fd837.
