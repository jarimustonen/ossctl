---
created: 2026-08-20
updated: 2026-08-20
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
---

# Cargo-dist verifier reports existing GitHub Releases missing

## Description

Cargo-dist verifier reports existing GitHub Releases missing

## Context

I was releasing `project-canon` 0.6.0 after adding binary-owned distribution for the canonical `cli-canon` skill.

Repository: https://github.com/jarimustonen/project-canon
Feature commit: `10a6cd602c02c8e11d7f71d4c8fedc4c9e39ba7c`
Release commit/tag target: `005fafa5ae33ea955a83b76aeefb403f2ff3d6b3` / `v0.6.0`
Installed ossctl: `0.9.0`, commit `4bd2ae389627c39ee925c9969154d019628d7cd4`
Release contract target: `rust:project-canon-cli:gh-releases`, adapter `cargo-dist`

This report concerns the verifier, not cargo-dist itself: cargo-dist completed successfully and the GitHub Release was publicly visible with assets.

## Reproduction and exact run

After manually preparing the 0.6.0 release commit, I sealed:

```sh
ossctl release plan --json
```

Plan id:

`de0d83794b6a4afeb9e5d6c0d022c57dcdadc28496723a7f939907f96a4ba13d`

Then ran:

```sh
ossctl release cut --plan de0d83794b6a4afeb9e5d6c0d022c57dcdadc28496723a7f939907f96a4ba13d --json
```

Run id:

`01M0CDRH6CBM574MRFMQTCR9W2`

The run successfully completed:

- dry_run: ok
- build: ok
- publish: both crates.io targets published
- tag: `v0.6.0` created and pushed
- GitHub Release delegated to cargo-dist
- dist: ok
- crates.io verification: both `matches`

The verifier then waited about 20 minutes and recorded:

```json
{"target":"rust:project-canon-cli:gh-releases","outcome":"missing"}
```

The run ended with:

```text
verify-phase failed on target `rust:project-canon-cli:gh-releases`: rust:project-canon-cli:gh-releases is missing at its destination
```

## External evidence that the target existed

At the time ossctl reported `missing`:

- GitHub Release existed and was published: https://github.com/jarimustonen/project-canon/releases/tag/v0.6.0
- cargo-dist workflow run `32226769657` completed successfully: https://github.com/jarimustonen/project-canon/actions/runs/32226769657
- Release contained `dist-manifest.json`, shell installer, Homebrew formula, source archives, checksums, and macOS/Linux arm64+x86_64 archives.
- crates.io had both `project-canon-core 0.6.0` and `project-canon-cli 0.6.0`.
- Homebrew tap reported stable `0.6.0`, and `brew info` showed 0.6.0 installed.

`gh release view v0.6.0 --repo jarimustonen/project-canon` returned a non-draft, non-prerelease release with `publishedAt: 2026-08-19T07:15:41Z` and the assets above.

Because all external targets had landed, I had to mark the journal run abandoned with this reason:

```text
Release fully landed: both crates, v0.6.0 GitHub Release assets, successful cargo-dist workflow, and Homebrew 0.6.0 verified; ossctl gh-releases adapter returned a false missing result
```

## Repeated historical evidence

This was not unique to 0.6.0. `ossctl release list --json` showed three earlier project-canon runs stuck `in_progress` for the same delegated GitHub Release verification shape:

- 0.3.3: `01M07ECC8ZFQ58DH63ZDMKJPPP`
- 0.4.0: `01M08H40373DF571PXY979934D`
- 0.5.0: `01M08P4D4HK25MRQXDE0XDW9NJ`

For each, `ossctl release verify <run-id> --json` reported both crates.io targets as `matches` and the cargo-dist `gh-releases` target as `missing`, even though the corresponding GitHub Release exists publicly. I reconciled and abandoned these stale runs before starting 0.6.0 because they violated the single-active-cut invariant.

## Observed

The cargo-dist delegated GitHub Release verifier gives a false `missing` result after a successful workflow and published release. It leaves fully published releases in `in_progress`, blocks the single-active-cut invariant, and forces operators to abandon otherwise successful runs.

## Expected

For a delegated cargo-dist `gh-releases` target, verification should detect the matching published GitHub Release/tag (and, if required by the contract, expected release assets) and mark the target `matches`. A fully landed run should reach terminal completed state without manual abandonment.

If the adapter cannot verify GitHub at that moment, it should report `unknown` with diagnostic evidence rather than a definitive false `missing`.

## Analysis pointers

- Inspect how the gh-releases registry adapter derives repository coordinates, package name, tag, and version for a cargo-dist delegated target.
- `project-canon-cli` is the Cargo package, `project-canon` is the repository and installed binary/formula name, and the tag is `v0.6.0`; a package-vs-repository-name mismatch may be relevant.
- Compare verifier requests with `gh release view v0.6.0 --repo jarimustonen/project-canon` or the equivalent GitHub Releases API endpoint.
- The project-canon release journal retains the run ids and event history listed above.
