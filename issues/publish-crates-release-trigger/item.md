---
created: 2026-08-06
updated: 2026-08-10
type: bug
status: fixed
priority: high
commits:
- hash: 1e186968a9270a8dac6f577e5d32e18c2297249e
  summary: trigger publish-crates on version-tag push (fix release:published dead trigger)
closed: 2026-08-10
---

# publish-crates.yml never auto-fires: release:published dead for cargo-dist releases

## Description

The generated/reference `.github/workflows/publish-crates.yml` triggers on `on: release: published`. But cargo-dist's release.yml creates the GitHub Release using the default `GITHUB_TOKEN`, and GitHub does NOT emit workflow events (incl. `release`) for actions taken by GITHUB_TOKEN. So on a real cargo-dist-driven release, publish-crates.yml never runs — crates.io is silently not published.

Observed: `gh run list --workflow 'Publish to crates.io'` for both ossctl and issuectl shows EVERY run is `event=workflow_dispatch` — the `release` trigger has never fired once. crates.io publishing has been happening only via manual dispatch.

Expected: a tag push publishes to crates.io automatically alongside binaries+Homebrew.

Fix (verified working in example-org/glasspad): trigger on the version-tag push instead —
```
on:
  push:
    tags: ['v[0-9]+.[0-9]+.[0-9]+*']
  workflow_dispatch: ...
```
and change the publish step's `if` from `github.event_name == 'release'` to `== 'push'`. Runs in parallel with release.yml (crates publish needs only the tagged source). Affects the ossctl AND issuectl repo workflows (and any oss-* skill that emits this template).

## Decision (maintainer, 2026-08-10) — Option A: fix the generated template (ossctl's job)

**Chosen: A — this is ossctl's job.** If the `/oss-*` family generates a CI workflow template, it must
generate a *working* one. Apply the verified fix (trigger on the version-tag `push`, change the publish
step's `if` to `== 'push'`) to whatever ossctl emits/references as the crates-publish workflow — find
the source of truth (a template resource / bundled skill / the reference `.github/workflows/publish-
crates.yml`) and fix it there so every future generated project inherits the working trigger. SCOPE
NOTE: updating issuectl's OWN repo workflow file is a separate, homebase/issuectl-repo concern — do NOT
touch other repos from here; only fix what ossctl generates + ossctl's own reference workflow.
