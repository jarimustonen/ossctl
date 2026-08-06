---
created: 2026-08-06
updated: 2026-08-06
type: bug
status: in-progress
priority: high
epic: ossctl-phase4-build
commits:
- hash: bfb05d3
  summary: build-phase --no-verify + faithful dry-run preflight
---

# release cut build-phase can't package a dependent crate whose =-pinned dep isn't published yet

## Description

_Source: first engine-driven cut of ossctl itself (0.2.0), run 01KZB29WPXV11XK5K8SMTA4FGA._

## Problem

`ossctl release cut` fails in the **build phase** on a multi-crate workspace whose dependent
crate pins its workspace dep by exact version. Cutting ossctl 0.2.0 (targets `ossctl-core`
then `ossctl` on crates.io) failed at `build-all`:

```
build-phase failed on target `rust:ossctl:crates.io`:
`cargo package --registry crates-io -p ossctl` failed (exit 101):
  failed to select a version for the requirement `ossctl-core = "=0.2.0"`
  candidate versions found which didn't match: 0.1.2, 0.1.1, 0.1.0
  location searched: crates.io index
```

`ossctl` depends on `ossctl-core = "=0.2.0"`. `cargo package -p ossctl` runs a **verify build**
that resolves deps against the crates.io **index**, where `ossctl-core 0.2.0` does not exist yet
(it is only *published* in the later publish phase). So the dependent crate cannot be packaged
until its dep is published — but the phase barrier does **build-ALL → publish-ALL**, packaging
`ossctl` before `ossctl-core` is ever published.

## Why dry-run didn't catch it

The `dry-run` phase passed for all four targets (incl. `rust:ossctl:crates.io`) — `cargo publish
--dry-run` / the adapter dry_run does not perform the same index-resolving verify build that
`cargo package` does in the build phase. So the barrier's own dry-run-all is not a faithful
preflight of the build phase for this case.

## Impact

Blocks the engine-driven cut of ossctl itself (and any multi-crate workspace with `=`-pinned
internal deps — the exact shape `/oss-init` emits). **Failure is SAFE**: it happens before
publish-all, so nothing was published, no tag was created; the run abandons cleanly.

## Proposed fix (needs design + review — touches the phase-barrier/coordinator + cargo adapter)

The build phase must not require a dependent's `=`-pinned workspace dep to be on the index before
its own dep is published. Options:
1. Build phase uses `cargo package --no-verify` (produce the `.crate` without the index-resolving
   verify build); the publish phase's `cargo publish` still verifies, and by then the dep is
   published + index-waited, so it resolves. Simplest.
2. For same-ecosystem dep-ordered cargo targets, interleave build→publish per target in dependency
   order instead of a strict build-all → publish-all barrier (a cargo-ecosystem-specific
   execution shape).
3. Make the dry-run phase faithfully exercise the build-phase package-verify so this fails at
   dry-run (before any side effect), regardless of which execution fix is chosen.

Prefer (1) + (3): `--no-verify` in build, and a dry-run that mirrors the real build so the barrier
never enters publish-all on a plan that would fail to build. Preserve ADR-0002 phase-barrier
invariants and ADR-0004 (one target = one publish unit).
