//! Readiness scoring over the normalized contract + detected facts (ADR-0001 §3).
//!
//! Pure rules producing a gap-report: the gated core (README + LICENSE + CI)
//! plus the tier-scaled canon. Read-only — no repo writes. Feeds `ossctl audit`
//! and the `/oss-readiness` skill. Stub at founding; lands in the
//! `audit-command` unit.
