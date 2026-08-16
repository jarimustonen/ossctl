---
created: 2026-08-05
updated: 2026-08-16
type: task
status: open
priority: normal
epic: ossctl-phase4-build
lane: release-safety
---

# release verify should confirm CI created a delegated GitHub Release

## Description

_Source: /llm-review of coordinator-release-vs-cargo-dist-ownership._

## Description

With Option 1 (coordinator delegates the GitHub Release to a target's CI, e.g. cargo-dist), an engine-driven cut reaches `RunStatus::Completed` once the coordinator's own work is done — but the engine cannot synchronously observe whether the tag-triggered CI workflow actually created and finalized the Release. If `release.yml` fails (builder timeout, expired token, workflow bug, broken tag), there is no Release and no binaries, yet `ossctl` reports success and the tag/publishes are irreversible.

This is an accepted consequence of Option 1 (the chosen direction), NOT a bug in the delegation change — the issue `coordinator-release-vs-cargo-dist-ownership` explicitly scoped 'add an end-to-end check before the first real engine-driven 0.2.0 cut' as a pre-cut step, and ADR-0002's addendum documents it. This issue tracks turning that manual check into tooling.

## Proposed

`ossctl release verify <run_id>` should recognize a `github_release_delegated` tag and, for it, query GitHub (`gh release view <tag>` / API) to assert the Release now exists (and optionally that expected cross-platform assets are present), rather than assuming success from the delegation event alone. Distinguish: coordinator-created (Release must exist), delegated+observed (exists), delegated+pending (not yet), delegated+failed (absent after a timeout / explicit operator check).

Currently `release verify`/`reconcile` do NOT key on `TagState.github_release` at all (only `release show` display does), so this is additive, not a fix to existing broken behavior.
