---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: open
priority: high
epic: ossctl-phase4-build
related: ['@cargo-publish-receipt-provenance-resume-safety']
lane: release-hardening
lane_seq: 20
blocked_by: ['@registry-published-release-port']
---

# Journal the .crate digest at build time instead of re-deriving it on resume

## Description

Split out of @cargo-publish-receipt-provenance-resume-safety — the core slice, isolated so it can be done properly in one unit.

The defect: the resume path re-packages the crate and re-derives its digest. `cargo package` is deterministic only under a fixed toolchain + source + index state, so a resume under an upgraded cargo can produce different bytes for the same sealed commit, yielding a spurious `DigestMismatch` that wedges a LEGITIMATE resume. Safe for same-session dogfood resumes, regressive for the generic cross-toolchain /oss-* case.

The fix: journal a PRE-publish `artifact_prepared` fact carrying the digest of the exact `.crate` bytes (a receipt-after-upload alone misses the crash window). On resume, compare the JOURNALED digest against the registry cksum. The resume-time re-package and `intended_crate_digest` derivation then go away entirely — which also removes the `cargo package`-inside-`publish()` phase-purity wrinkle and its yanked-sibling failure mode.

Also: resume must consult the prepared fact before converting `Missing` to 'publish', closing the duplicate-irreversible-upload window during publish-to-index lag.

Known blast radius (a prior attempt stalled here — budget for it): a journal v5 event + reducer, coordinator write-ahead ordering, the cargo adapter's prepare/publish flow, and resume classification. Expect BROAD churn in the existing test fixtures; updating them is part of the unit, not a reason to stop. Journal version bump must be deliberate and must keep `release resume` working against runs journaled at v4.

Fail CLOSED on every ambiguity: missing prepared fact, digest mismatch, registry outage. This is the irreversible-publish path.
