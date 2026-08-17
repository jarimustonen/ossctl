---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: high
---

# release tag phase creates the GitHub Release, breaking cargo-dist hosting and skipping the Homebrew publish

## Description

## Summary

For a repo whose binary-distribution backend is **cargo-dist**, `ossctl release cut`'s `tag`
phase creates the **GitHub Release** in addition to pushing the tag. cargo-dist's tag-triggered
workflow then tries to create the same release, fails with `a release with the same tag name
already exists`, and every job downstream of `host` — including
**`publish-homebrew-formula`** — is skipped.

Net effect: crates.io and the tag succeed, but the release ships **no binaries and no Homebrew
formula**. The cut reports success.

Found cutting `issuectl` 0.14.1 with ossctl **0.6.1** (commit `2846d66`).

## Observed

`ossctl release cut` reported all six phases green:

    → tag
      tag created: v0.14.1
      tag pushed: v0.14.1
      release: v0.14.1 (https://github.com/jarimustonen/issuectl/releases/tag/v0.14.1)
    ✓ tag complete
    → dist
    ✓ dist complete
    release complete — version: 0.14.1  tag: v0.14.1  published 2 target(s)

The cargo-dist workflow fired on the same tag and failed:

    host   Create GitHub Release   a release with the same tag name already exists: v0.14.1
    ##[error]Process completed with exit code 1

    plan                                          success
    build-local-artifacts (aarch64-apple-darwin)  success
    build-local-artifacts (x86_64-pc-windows-msvc) success
    build-local-artifacts (aarch64-unknown-linux-musl) success
    build-local-artifacts (x86_64-unknown-linux-musl)  success
    build-global-artifacts                        success
    host                                          failure
    publish-homebrew-formula                      skipped
    announce                                      skipped

Every binary built fine. The only thing that failed was creating a release object that ossctl
had already created **as an empty shell**:

    $ gh release view v0.14.1 --json assets --jq '.assets|length'
    0
    $ gh release view v0.14.0 --json assets --jq '.assets|length'     # previous, manual route
    14

## Confirmation of the cause

Deleting the ossctl-created release (leaving the git tag `v0.14.1` = `7fefc71` untouched) and
re-running the failed jobs made it pass immediately:

    host                       success
    publish-homebrew-formula   success

    $ gh release view v0.14.1 --json assets --jq '.assets|length'
    15
    $ # tap formula
    version "0.14.1"

So the release object created by the `tag` phase is the sole blocker. Nothing else changed.

## Why this is easy to miss

The previous release of the same repo (0.14.0) was cut on the **manual** path, which only pushed
a tag and never created a release — so cargo-dist's `host` job created it and everything worked.
The regression appears **only** on the engine path, and only in repos that delegate hosting to
cargo-dist. The engine's own output says `✓ dist complete` and `release complete`, so an operator
who trusts the cut has no signal that the binary and Homebrew legs were dropped.

That last point is the important one: **the cut reports success while the most user-visible half
of the release silently did not happen.**

## Expected

One of:

1. When the contract/repo delegates hosting to cargo-dist, the `tag` phase should push the tag
   and **stop** — let cargo-dist create the GitHub Release.
2. Or create the release only if one does not exist, and make it tolerable for cargo-dist to
   adopt/complete it rather than colliding.
3. At minimum, the `dist` phase must **verify** rather than assume: it currently prints
   `✓ dist complete` without checking that the downstream workflow produced assets. It should
   fail (or loudly warn) when the tag's release ends up with zero assets, so a dropped binary /
   Homebrew leg cannot be reported as a completed release.

(3) matters independently of (1)/(2): this repo's operating policy calls the Homebrew leg the
most important target, and it was dropped without a single error surfacing to the operator.

## Related

- `@release-bump-plan-uncuttable` — separate 0.6.1 bug hit during the same cut.
