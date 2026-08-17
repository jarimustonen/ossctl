---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: high
lane: release-safety
lane_seq: 1
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

## Comments

### 2026-08-17T05:47:46Z · @claude

TRIGGER IDENTIFIED (2026-08-17), and it narrows the fix.

Compared the two contracts. ossctl's own contract declares FOUR targets including `{registry: gh-releases, adapter: cargo-dist}` and `{registry: homebrew, adapter: homebrew-tap}`; its cuts print 'release delegated to CI (cargo-dist)' and do NOT create the release. issuectl's contract declares only its TWO crates.io targets — no gh-releases target, no homebrew target — even though the repository genuinely uses cargo-dist and a Homebrew tap.

So the engine did not misbehave against a declared delegation; it had no idea a delegation existed. With no gh-releases target in the contract it fell back to creating the release itself, and collided with the cargo-dist workflow that the repository really does run.

This means fix options (1) and (2) as written cannot work on their own: they assume the engine can tell that hosting is delegated, and for this contract shape it cannot. The engine must either DETECT the cargo-dist backend from the repository (a `dist-workspace.toml` with `ci = "github"` — the same cross-read @contract-validate-warn already added for the Homebrew tap), or refuse to create a release in a repository that has a cargo-dist workflow, or fall back to option (3), verification after the fact.

Note the family resemblance: this is the same defect shape as @intake-feature-ossctl-73e870268475 (a tap declared in the distribution block but absent from targets, silently dropped) and @contract-validate-warn. A contract that under-declares its real distribution surface produces a green cut that quietly does the wrong thing. A validation warning for 'this repo has a cargo-dist workflow but no gh-releases target' would have caught this before the cut, and is probably the cheapest half of the fix.

EXPOSURE CHECK for the pending release queue: glasspad's contract DOES declare both the gh-releases/cargo-dist and homebrew targets, matching ossctl's shape — so glasspad is NOT exposed to this bug and its planned 0.15.0 cut is safe on this axis. issuectl's contract needs its gh-releases and homebrew targets added; that is an issuectl-repo task, separate from this fix.

### 2026-08-17T06:10:29Z · @claude

DESIGN LANDED (2026-08-17): design.md in this issue directory covers the whole release-safety cluster. This issue's fix is D4 (facts-to-contract distribution cross-check: warn at plan, hard-refuse at cut when the repo has a cargo-dist release workflow or a declared homebrew_tap that the targets list omits) + D3 (mandatory post-cut verify phase, so a dropped leg can never report green). Option (1) of the Expected list is unreachable for this contract shape (the engine cannot know about an undeclared delegation), confirming the 2026-08-17 trigger comment.

