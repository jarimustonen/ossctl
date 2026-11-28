---
created: 2026-11-28
updated: 2026-11-28
type: bug
reporter: pi-conductor
status: untriaged
priority: normal
provenance: agent
source_ref: agent:taskfleet-conductor/release:v0.23.0/dist-manifest-missing
---

# Green cargo-dist release omits dist-manifest.json; verify stops on gh-releases

## Description

## Observed

Two consecutive Taskfleet releases (Open-Habitat/taskfleet v0.22.0 on 2026-11-26 and v0.23.0 on 2026-11-28, run 46120531630) published a GitHub Release without `dist-manifest.json`, although every job of cargo-dist 0.33.0's generated `release.yml` was green. `release cut` then stopped in verify with `rust:taskfleet:gh-releases is missing at its destination`; crates.io and Homebrew matched. v0.21.0 and v0.21.1 (same day, same workflow file; `release.yml` did not change between v0.21.1 and v0.22.0) had the asset.

## Likely cause

In the `host` job, `actions/upload-artifact@v7` uploads `artifacts-dist-manifest` and the next step, `actions/download-artifact@v8` with `pattern: artifacts-*`, runs about one second later. In run 46120531630 it reported "Total of 5 artifact(s) downloaded" (plan manifest, three local builds, global), without the artifact uploaded a second earlier in the same job, so `gh release create … artifacts/*` had no `dist-manifest.json`. This looks like the artifact listing no longer seeing a just-finalized upload from the same job (a GitHub-side change between 26.11. 06:36 and 07:01 UTC), not a repository change. Not confirmed against GitHub or cargo-dist issues.

## Recovery used both times

`gh run download <run> -n artifacts-dist-manifest`, `gh release upload <tag> dist-manifest.json`, then the wrapper's `resume`; `verify` matched all four targets.

## Expected

A green release workflow never leaves the gh-releases target unreconcilable. Options: the engine recognises a release whose only missing asset is `dist-manifest.json` and offers or performs the recovery from the host job's artifact; or the generated workflow copies `dist-manifest.json` into `artifacts/` directly instead of relying on the round trip (needs a cargo-dist-compatible way, since `dist generate --check` rejects hand edits); or check upstream cargo-dist for a fix.
