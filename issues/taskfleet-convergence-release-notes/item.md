---
created: 2026-09-06
updated: 2026-09-06
type: task
status: open
priority: high
related: ['@taskfleet-zero-legacy-repository']
---

# Prepare Shipshape Taskfleet convergence release notes

## Problem

The completed repository-wide Taskfleet identity convergence is production-ready and exact-main CI is green, but the curated CHANGELOG `[Unreleased]` section is empty. Cutting a patch release now would omit the user-visible clean-break and delegated-release reconciliation improvements.

## Required work

Using the repository's marker-anchored changelog ownership rules, add concise `[Unreleased]` entries covering removal of the remaining retired product identity from maintained Shipshape surfaces and reliable observation of repository-local cargo publish helpers/current cargo-dist Homebrew output. Do not change application code, versions, tags, contracts, generated distribution files, or external repositories. Keep the repository-wide canonical-identity scan green. Run focused changelog/contract checks and the appropriate documentation/format gate.

## Acceptance Criteria

- [ ] `[Unreleased]` accurately describes both shipped changes without naming the retired identity.
- [ ] Changelog markers remain valid and the release engine can plan a patch release.
- [ ] Canonical identity scan remains at zero retired references.
- [ ] Required focused gate passes.
