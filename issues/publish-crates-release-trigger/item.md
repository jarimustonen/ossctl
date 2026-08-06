---
created: 2026-08-06
updated: 2026-08-06
type: bug
status: open
priority: high
---

# publish-crates.yml never auto-fires: release:published dead for cargo-dist releases

## Description

The generated/reference `.github/workflows/publish-crates.yml` triggers on `on: release: published`. But cargo-dist's release.yml creates the GitHub Release using the default `GITHUB_TOKEN`, and GitHub does NOT emit workflow events (incl. `release`) for actions taken by GITHUB_TOKEN. So on a real cargo-dist-driven release, publish-crates.yml never runs — crates.io is silently not published.

Observed: `gh run list --workflow 'Publish to crates.io'` for both ossctl and issuectl shows EVERY run is `event=workflow_dispatch` — the `release` trigger has never fired once. crates.io publishing has been happening only via manual dispatch.

Expected: a tag push publishes to crates.io automatically alongside binaries+Homebrew.

Fix (verified working in jarimustonen/glasspad): trigger on the version-tag push instead —
```
on:
  push:
    tags: ['v[0-9]+.[0-9]+.[0-9]+*']
  workflow_dispatch: ...
```
and change the publish step's `if` from `github.event_name == 'release'` to `== 'push'`. Runs in parallel with release.yml (crates publish needs only the tagged source). Affects the ossctl AND issuectl repo workflows (and any oss-* skill that emits this template).
