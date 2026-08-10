---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: open
priority: high
---

# publish-crates.yml dep-order step not idempotent: cargo publish -p ossctl-core dies with exit 101 on 'already exists'

## Description

The v0.2.4 release marked CI red even though **both crates published fine**
(crates.io confirmed `ossctl@0.2.4` and `ossctl-core@0.2.4`). The
`Publish to crates.io` workflow run
[31390837102](https://github.com/jarimustonen/ossctl/actions/runs/31390837102)
failed the `Publish (dep-order)` step:

```
error: crate ossctl-core@0.2.4 already exists on crates.io index
##[error]Process completed with exit code 101.
```

## Root cause

`.github/workflows/publish-crates.yml` → job `cargo publish (workspace, dep-order)`
→ step `Publish (dep-order)` runs under `set -euo pipefail`:

```sh
cargo publish -p ossctl-core          # <-- no already-exists tolerance
for attempt in 1..6; do cargo publish -p ossctl && exit 0; sleep 30; done
```

The retry/idempotency guard only wraps `ossctl` (the leaf). The first line,
`cargo publish -p ossctl-core`, has **no such guard**: if ossctl-core@<ver> is
already on the index (because the tag triggered the workflow more than once, or a
prior attempt already landed core), `cargo publish` exits non-zero, `set -e`
aborts the whole step → the release shows failure despite a fully-successful
publish.

Both `publish-crates.yml` and `release.yml` trigger on the same `v*` tag, so a
second publish trigger for one tag is a live scenario.

## Fix

Make each `cargo publish` idempotent: treat "already exists on crates.io index"
as success. E.g. wrap both publishes in a helper that greps the failure output
for `already exists` and returns 0, or check the index first. Apply the same
tolerance to the `ossctl-core` line, not just the `ossctl` retry loop.

## Impact

Spurious red release runs, notification noise, and false "release failed"
signal — the release actually succeeded every time this has fired.

