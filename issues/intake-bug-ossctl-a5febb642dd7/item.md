---
created: 2026-08-23
updated: 2026-09-02
type: bug
reporter: jari
status: open
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:homebase-wrapup-ossctl-cargo-publish-detection-20260823
lane: release-preflight
---

# Release plan misses tag-triggered Cargo publish workflow

## Description

Release plan misses tag-triggered Cargo publish workflow

## Observed

From a clean orchestratectl main checkout with `.github/workflows/publish-crates.yml` present, this command:

```sh
scripts/ossctl-release.sh plan patch
```

sealed a valid four-target plan but emitted:

```text
cargo-publish-ci delegates crates.io publication for octl-core, orchestratectl to CI, but no tag-triggered Cargo publish workflow was detected under .github/workflows; no directly inspectable Cargo publish path was found in the detected tag-triggered workflows (release.yml).
```

The repository's `.github/workflows/publish-crates.yml` contains `on: push: tags: ['v[0-9]+.[0-9]+.[0-9]+*']` and directly runs `cargo publish -p octl-core` followed by `cargo publish -p orchestratectl`. The resulting v0.5.1 tag later triggered that workflow successfully, and ossctl verification reported both crates.io targets as `matches`.

## Expected

`ossctl release plan` should detect the checked-in tag-triggered Cargo publish workflow and not warn that no directly inspectable Cargo publish path exists. Detection should cover this valid YAML trigger and direct shell publish form without weakening the warning for genuinely missing CI publication paths.

## Evidence

- Plan id: `ee4cdfb863909c73587a9294ccdc613c5b494d81fab061e6b8fd101192b757b3`
- Release journal: `01M0QA6BTN55D9K1YB7QGS83DW`
- Release commit: `f0c52ab232706fb480a51bfd45f2171c6b7aa056`
- Publish workflow run: `32640599433` (success)
