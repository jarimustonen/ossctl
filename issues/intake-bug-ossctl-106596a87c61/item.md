---
created: 2026-09-03
updated: 2026-09-03
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:issuectl-wrapup
source_ref: agent:issuectl-wrapup/reporter:jari/id:issuectl-wrapup-2026-09-03-dist-preflight
---

# Release cut checks cargo-dist only after bump

## Description

Release cut checks cargo-dist only after bump

## Description

`shipshape release cut` can enter a journaled release, create the isolated version/changelog bump commit, complete package dry-runs, and only then discover that the pinned cargo-dist executable (`dist`) is unavailable.

## Observed

In issuectl's v0.17.1 release, the approved command was:

```sh
shipshape release cut --plan 6bbf8b19dcef6535441984e85bb13c6920398c9caf9740d1bad8bd0ae1152e04 --json
```

The run completed the `bump` and `dry_run` phases, then failed in `build` with:

```text
run 01M1HH410VMB237JZS47BZVXBB: build-phase failed on target `rust:issuectl:gh-releases`: cannot run `dist build`: No such file or directory (os error 2)
```

Nothing had been published or tagged, but the release was already journaled and carried an isolated bump commit. The same run resumed successfully after placing a checksum-verified disposable cargo-dist 0.28.2 binary on PATH; that version exactly matched `dist-workspace.toml`.

Persistent cargo-dist installation is intentionally not part of this machine's convergence policy, so requiring an ambient `dist` without a preflight or disposable provisioning path is not reliable.

## Expected

Before creating the release run or applying its bump phase, Shipshape should validate every required external executable, including the cargo-dist version pinned by `dist-workspace.toml`. It should either:

1. fail before mutation with an actionable error naming the missing executable and required version; or
2. provision and verify the pinned cargo-dist release in a disposable run-local location.

The preflight should prevent a known missing local build dependency from being discovered only after release mutation has begun. Resume semantics must remain available for genuinely transient failures after a run starts.
