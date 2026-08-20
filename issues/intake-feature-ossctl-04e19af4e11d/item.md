---
created: 2026-08-16
updated: 2026-08-20
type: feature
reporter: jari
status: duplicate
priority: normal
labels:
- via:agent-homebase-wrapup
related: ['@oss-dist-channel-generator']
closed: 2026-08-16
---

# No standalone Homebrew-formula (re)generate/push; generated model assum…

## Description

No standalone Homebrew-formula (re)generate/push; generated model assumes prebuilt formula, family uses source-build

## Context

Cutting project-canon 0.1.1 with `/oss-release` (ossctl 0.2.2), two Homebrew gaps showed up:

## 1. No standalone formula step
The Homebrew formula is only produced/pushed by the release-cut engine's `dist` phase — there is
no `ossctl dist formula` (or similar) to (re)generate and push the tap formula on its own. Once a
cut partially fails (see release-resume-unimplemented) or you finish a release by hand, there is no
way to drive just the formula step; you must write and push `Formula/<name>.rb` manually.

## 2. Generated model diverges from the actual family convention
`ossctl dist generate` deliberately EXCLUDES `homebrew` from the cargo-dist `[dist] installers`
(warning: "the 'homebrew' installer is published by ossctl's Homebrew tap adapter (post-tag, once
the release tarball sha256 exists)"), i.e. it assumes a PREBUILT-BINARY formula pushed by an ossctl
tap adapter. But the actual sibling taps (homebrew-ossctl, homebrew-issuectl, homebrew-orchestratectl)
ship a SOURCE-BUILD formula: `depends_on "rust" => :build` + `cargo install` from the GitHub source
tarball. So the generated model and the real convention disagree, and neither the tap adapter nor a
source-build formula generator is actually reachable standalone today.

## Expected
Either (a) a standalone `ossctl` subcommand to render + push the tap formula for a given tag
(choosing source-build vs prebuilt per contract), or (b) have cargo-dist's release.yml publish the
formula in CI (add `homebrew` to installers + tap + a tap-push token) so it's automatic — matching
how the sibling taps say "Formula published by cargo-dist on release."

## Env
ossctl 0.2.2. Related: release-cut-ignores-version, release-resume-unimplemented (same release).

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

The standalone Homebrew formula regeneration need belongs in the retained oss-dist channel generator work.
