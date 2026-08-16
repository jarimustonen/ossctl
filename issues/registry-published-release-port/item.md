---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@cargo-publish-receipt-provenance-resume-safety']
lane: release-hardening
lane_seq: 10
---

# Consolidate crates.io sparse-index probe into one published_release call

## Description

Split out of @cargo-publish-receipt-provenance-resume-safety (that issue proved too large for one unit). Foundation slice, no journal changes.

Scope:
1. Collapse the two sparse-index fetches (`published_versions` + `published_checksum`) into a single `published_release(...) -> {present, cksum}` RegistryQuery port call — one round trip, fewer transient failure modes.
2. Add a distinct `AdapterError` variant for 'cannot self-authenticate' (the crate IS published but local digest derivation failed), today surfacing as a raw `AdapterError::Command`. Include operator guidance in the message.
3. Minor: `entry.vers == version` is string- not SemVer-equality (build metadata); `parse_sparse_checksum` takes the first match with no duplicate-record detection.

Fail closed on every ambiguity (outage, malformed index line). Keep the diff to the port + adapter error surface; the journaled-digest rework is @cargo-publish-digest-journaled.
