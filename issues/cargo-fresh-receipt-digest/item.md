---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@cargo-publish-digest-journaled']
lane: release-hardening
lane_seq: 30
blocked_by: ['@cargo-publish-digest-journaled']
---

# Fresh cargo publishes must record a digest on the receipt

## Description

Split out of @cargo-publish-receipt-provenance-resume-safety. Depends on @cargo-publish-digest-journaled (build on top of the journaled-digest flow).

A fresh (non-resume) publish still records `digest: None`, and `verify_via_registry` never populates `RemoteObservation.remote_digest` — so `classify_receipt`'s `Conflicts` branch is undetectable for fresh receipts, and the ADR-0003 reconcile state table cannot do its job.

Fix: after `confirm_self_published`, read the registry checksum and record it on the receipt; populate `remote_digest` in verify so Conflicts becomes detectable. Fail closed on an unreadable checksum.
