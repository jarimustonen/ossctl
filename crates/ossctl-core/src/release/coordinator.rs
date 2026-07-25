//! Phase-barrier coordinator: ordering, phase barriers, and tag ownership
//! (ADR-0002).
//!
//! Drives all ecosystem adapters through the barriers dry-run-all → build-all →
//! publish-all → tag-once, with tagging owned by the coordinator alone (never an
//! adapter). Stub at founding; lands in the `release-engine` unit.
