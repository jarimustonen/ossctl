//! Event-sourced release journal (ADR-0003).
//!
//! Append-only JSONL events under `git-common-dir/ossctl/releases/<run_id>/`,
//! with a reducer folding them into resumable run state. The durable record
//! `release resume`/`verify`/`show` read back. Stub at founding; lands in the
//! `release-engine` unit.
