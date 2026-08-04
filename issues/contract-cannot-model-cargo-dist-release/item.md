---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: in-progress
priority: normal
commits:
- hash: 2ce27d7
  summary: first-class distribution block + normalize/validate + SEAL_VERSION bump + tests
---

# OSS-RELEASE contract can't model a cargo-dist release (binaries + installer + Homebrew tap) alongside registry publishes

_Source: OSS-RELEASE.md targets/adapter model_

## Description

Surfaced running `/oss-init` on the **issuectl** repo.

## Observed
issuectl's release engine is **cargo-dist** (tag-triggered `.github/workflows/release.yml`) which produces: multi-platform GitHub-Release binaries, a shell installer, and a Homebrew formula pushed to a tap (`jarimustonen/homebrew-issuectl`) — PLUS a separate crates.io publish (`publish-crates.yml`). The contract's `targets: [{ecosystem, package, registry, adapter}]` model only represents **registry** publishes; `adapter` allows `cargo-dist` but there is no first-class field for the **binary-distribution + tap + installer** layer.

## Expected / suggestion
A way to express a cargo-dist-style release in the machine contract, e.g.:
- a `binary`/`gh-releases` distribution target type (distinct from a registry target), and/or
- a release-level `adapter: cargo-dist` with sub-config for `installer`, `homebrew_tap`, and `registry_publish` split.

## Impact
Had to record the whole binary-distribution reality as prose in the draft's `## Rationale` + `## Release notes` (with an explicit 'keep cargo-dist; do not let /oss-release-cut regenerate release.yml' note). Downstream `/oss-*` members reading the contract can't see the tap/installer, so they'd under-describe or risk clobbering the existing pipeline.
