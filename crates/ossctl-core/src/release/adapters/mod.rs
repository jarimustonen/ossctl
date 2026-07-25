//! Per-ecosystem release adapters behind the `ReleaseAdapter` trait (ADR-0002).
//!
//! One module per ecosystem (cargo, npm, …), each implementing the trait's
//! dry-run / build / publish steps that the [`super::coordinator`] drives
//! through the phase barriers. An enum-backed registry selects adapters; the
//! coordinator owns tagging, never the adapter. Stub at founding; the trait and
//! first adapters land in the `release-engine` unit.
