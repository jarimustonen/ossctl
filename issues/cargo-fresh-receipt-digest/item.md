---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: wontfix
priority: normal
epic: ossctl-phase4-build
related: ['@cargo-publish-digest-journaled']
lane: release-hardening
lane_seq: 30
blocked_by: ['@cargo-publish-digest-journaled']
closed: 2026-08-16
---

# Fresh cargo publishes must record a digest on the receipt

## Description

Split out of @cargo-publish-receipt-provenance-resume-safety. Depends on @cargo-publish-digest-journaled (build on top of the journaled-digest flow).

A fresh (non-resume) publish still records `digest: None`, and `verify_via_registry` never populates `RemoteObservation.remote_digest` — so `classify_receipt`'s `Conflicts` branch is undetectable for fresh receipts, and the ADR-0003 reconcile state table cannot do its job.

Fix: after `confirm_self_published`, read the registry checksum and record it on the receipt; populate `remote_digest` in verify so Conflicts becomes detectable. Fail closed on an unreadable checksum.

## Comments

### 2026-08-16T18:32:22Z · @claude

Closed as hypothetical (maintainer decision, stint #21). This finding came from an /llm-review panel, not from an observed failure. Review of the whole open issue base showed roughly 40% of it was defensive work of this class: cosmic-ray scenarios, checks layered on top of checks that already exist elsewhere, and hardening against hostile input in a path where the only actor is the maintainer breaking their own project. The scenario here has not occurred in 13 releases, the path is already structurally guarded (clean checkout of a sealed commit, dry-run before every upload, post-publish visibility confirmation), and both autonomous attempts at it stalled on its blast radius. Reopen if it is ever observed in the field.
