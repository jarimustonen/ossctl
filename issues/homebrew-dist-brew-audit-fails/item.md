---
created: 2026-08-06
updated: 2026-08-07
type: bug
status: in-progress
priority: high
epic: ossctl-phase4-build
commits:
- hash: 15dfbde
  summary: make homebrew dist leg self-sufficient (direct tap-write, drop bump-formula-pr audit dep)
- hash: 607407e
  summary: 'harden tap-write per multi-LLM review (sha 64-hex, existence+symlink guard, byte-compare idempotency, ruby # escape, pkg-name validation)'
---

# engine homebrew dist leg: brew bump-formula-pr fails on brew audit

## Description

_Source: the **first true engine dogfood cut**, `ossctl release cut` for 0.2.2, run
`01KZBMYYA02TKF7S7K97B5YY6C` (2026-08-06). The RegistryQuery fix
(`release-publish-registry-query-not-wired`) worked: the engine ran dry-run-all → build-all →
publish-all (both crates to crates.io) → tag → CI-delegation fully autonomously. It then failed in
the **`dist` phase on the LAST target**, `rust:ossctl:homebrew`._

## Problem

The homebrew adapter's dist step shells to:

```
brew bump-formula-pr --url https://github.com/jarimustonen/ossctl/archive/refs/tags/v0.2.2.tar.gz \
  --sha256 d10a707094e5c2d1a20064d7c43dfee3e2601bc9fa2576eed1a74ea96f1b2bdd -- ossctl
```

which exited 1 with:

```
Error: 1 problem in 1 formula detected.
Error: `brew audit` failed for ossctl!
```

`brew bump-formula-pr` runs `brew audit` on the bumped formula as part of its flow, and that audit
found one (unshown) problem, aborting the PR. The specific audit finding was not surfaced in the
engine's captured stderr.

## What was NOT the cause (ruled out this cut)

- **Not a sha mismatch.** The engine's `--sha256 d10a707…` **exactly matches** the real
  `curl -sL …/v0.2.2.tar.gz | shasum -a 256`. The GH auto-archive was byte-stable this time (cf.
  `homebrew-stable-source-tarball`, which remains a latent risk).
- **Not a missing archive / release.** The tag `v0.2.2` and its GitHub source archive existed.
- **Not a formula-structure regression.** The 0.2.1 formula is structurally identical and passed
  audit for the 0.2.0/0.2.1 manual bumps. Most likely a **transient/environmental `brew audit`**
  issue — `bump-formula-pr` bootstrapped a fresh rubocop/gem toolchain in the run (visible in the
  log), so a new/stricter style or `brew` core lint may have tripped on a formula content that is
  otherwise fine for tap use.

## Impact

The homebrew leg (the most important target per operating policy — must be cut, not dropped) was
**not** completed by the engine. Everything else in the cut shipped: crates.io ×2, tag, GH Release.
Completed **manually** by directly editing the tap `Formula/ossctl.rb` (url + sha → 0.2.2) and
pushing via the GitHub API (tap commit `ae7d54fc`) — bypassing `brew bump-formula-pr`/`brew audit`
entirely. So 0.2.2 is fully live on all four targets, but the engine's homebrew leg is not yet
self-sufficient.

## Fix (directions to consider)

1. **Capture the specific audit finding** — the engine swallowed it. Have the adapter surface
   `brew audit`'s actual message so the next failure is diagnosable, not a black box.
2. **Decide the audit stance.** Options:
   - Pass `--no-audit` (or `--force`) to `brew bump-formula-pr` so a cosmetic core-lint change can't
     block a tap bump the maintainer controls — the tap is a personal tap, not homebrew-core, so a
     full core audit is arguably too strict for the dist leg.
   - OR keep the audit but pin/relax the specific rule, and treat a genuine formula defect as a real
     failure.
3. **Consider dropping `brew bump-formula-pr` for a direct formula-file write** to the tap (the
   manual fallback used here: render `Formula/ossctl.rb` from url+sha, PUT via the API/git). This is
   deterministic, needs no local brew/gem toolchain on the cutting machine, and sidesteps the whole
   `bump-formula-pr` audit/PR apparatus. Likely the most robust long-term shape for a personal tap.
   Overlaps with `homebrew-publish-resume-idempotency` and `homebrew-create-resume-journaling`.

Preserve: real sha verification against the fetched archive, and fail-closed on a genuine
formula/archive problem (do not blindly force a broken formula).

