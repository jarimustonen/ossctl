//! Resume/verify reconciliation state table (ADR-0003).
//!
//! Reconciles a journaled run against remote registry state — the remote is
//! ground truth — to decide continue / read-only reconcile / terminal seal for
//! an interrupted, partially-irreversible run. Backs `release resume` and
//! `release verify`. Stub at founding; lands in the `release-engine` unit.
