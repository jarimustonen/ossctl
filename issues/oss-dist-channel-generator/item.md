---
created: 2026-08-06
updated: 2026-08-16
type: feature
status: in-progress
priority: normal
lane: oss-family
---

# No oss-* generator for gh-releases/cargo-dist + homebrew distribution channels

## Description

OSS-RELEASE.md's schema defines `gh-releases`/`cargo-dist` and `homebrew`/`homebrew-tap` target adapters, and `/oss-init` will write them into the contract — but NO member skill generates the artifacts those adapters imply. /oss-ci owns ci*.yml (PR gates) and explicitly NOT release*.yml; /oss-readme/-changelog/-contributing/-security-policy don't touch distribution. So a contract that declares a Homebrew/binary channel has no generator.

In practice (jarimustonen/glasspad 0.2.1) the whole cargo-dist channel had to be hand-wired by copying sibling repos: `dist-workspace.toml` (targets, installers=[shell,homebrew], tap, publish-jobs, github-custom-runners, github-attestations), `dist generate` → release.yml, `[profile.dist]` in Cargo.toml, creating the `homebrew-<tool>` tap repo, and setting HOMEBREW_TAP_TOKEN.

Proposal: an `/oss-dist` (or /oss-release-cut) generator that, from the contract's gh-releases/homebrew targets, emits dist-workspace.toml + release.yml, ensures [profile.dist], scaffolds the tap repo, and documents the required secrets — the missing distribution half of the family.

## Decision (Jari, 2026-08-10) — approved, next stint

**Approved.** Build it as a new `/oss-*` family member (likely via `/worktree-make-skill`, given it
composes the existing family + wraps `ossctl dist generate` + tap scaffold). Explicitly OK to land
in a LATER stint (not this round). Lower risk than the schema-fork decisions (no schema change).
