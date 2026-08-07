---
created: 2026-08-04
updated: 2026-08-07
type: improvement
status: done
priority: normal
epic: ossctl-phase4-build
related: ['@distribution-cross-platform-targets']
commits:
- hash: 0e0d3c6
  summary: 'feat(contract): warn on installer/platform OS mismatch'
- hash: ef3b837
  summary: 'review(contract): positional triple OS match + error-gate the crosscheck'
closed: 2026-08-07
---

# distribution: cross-validate installers against platforms (msi to windows, homebrew to darwin)

_Source: crates/ossctl-core/src/contract/normalize.rs (parse_distribution)_

## Description

_Source: /llm-review spin-off (F8) from `distribution-cross-platform-targets`_

## Problem

The `distribution` block can express an installer set that is inconsistent with its `platforms` set, and nothing flags it:

- `installers: [msi]` with no Windows (`*-windows-*`) triple in `platforms`
- `installers: [homebrew]` with no macOS (`*-apple-darwin`) triple
- (npm / shell installers are OS-agnostic — no constraint)

The generated installer then points at an artifact the release never builds — dead config. This is the same class as the existing `homebrew_tap`-set-without-`homebrew`-installer **warning** already in `parse_distribution`, so consistency argues for a matching installer↔platform check.

## Why its own issue / where it belongs

This is coverage-aware validation (it reasons about the OS each triple targets), which is exactly what the sibling `audit-cross-platform-gap` issue is being built to do (it already parses `platforms` for OS coverage). The installer→OS mapping (msi=Windows, homebrew=macOS/Linux, npm/shell=agnostic) needs its own small spec. Deliberately NOT bundled into the normalize-only parent diff, which is scoped to "field present + per-triple well-formed + default".

## Suggested shape

A warning (not a floor) in the normalizer or the audit: `installers includes 'msi' but no windows target in platforms — the installer has nothing to install`.
