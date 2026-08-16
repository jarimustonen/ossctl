---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: wontfix
priority: normal
epic: ossctl-phase4-build
related: ['@cargo-publish-receipt-provenance-resume-safety']
lane: release-hardening
lane_seq: 10
closed: 2026-08-16
---

# Consolidate crates.io sparse-index probe into one published_release call

## Description

Split out of @cargo-publish-receipt-provenance-resume-safety (that issue proved too large for one unit). Foundation slice, no journal changes.

Scope:
1. Collapse the two sparse-index fetches (`published_versions` + `published_checksum`) into a single `published_release(...) -> {present, cksum}` RegistryQuery port call — one round trip, fewer transient failure modes.
2. Add a distinct `AdapterError` variant for 'cannot self-authenticate' (the crate IS published but local digest derivation failed), today surfacing as a raw `AdapterError::Command`. Include operator guidance in the message.
3. Minor: `entry.vers == version` is string- not SemVer-equality (build metadata); `parse_sparse_checksum` takes the first match with no duplicate-record detection.

Fail closed on every ambiguity (outage, malformed index line). Keep the diff to the port + adapter error surface; the journaled-digest rework is @cargo-publish-digest-journaled.

## Comments

### 2026-08-16T18:32:22Z · @claude

Closed as hypothetical (maintainer decision, stint #21). This finding came from an /llm-review panel, not from an observed failure. Review of the whole open issue base showed roughly 40% of it was defensive work of this class: cosmic-ray scenarios, checks layered on top of checks that already exist elsewhere, and hardening against hostile input in a path where the only actor is the maintainer breaking their own project. The scenario here has not occurred in 13 releases, the path is already structurally guarded (clean checkout of a sealed commit, dry-run before every upload, post-publish visibility confirmation), and both autonomous attempts at it stalled on its blast radius. Reopen if it is ever observed in the field.
