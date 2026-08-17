//! The resumable, journaled, per-ecosystem release-cut engine (ADR-0002).
//!
//! Owns the mechanics of `ossctl release plan/cut/resume/verify/show/list/
//! abandon`: a sealed content-addressed [`plan`], a phase-barrier
//! [`coordinator`] (dry-run-all → build-all → publish-all → tag-once, with
//! coordinator-only tagging), an event-sourced [`journal`] (ADR-0003), a
//! [`reconcile`] state table (remote is ground truth) with the [`resume`]
//! remote-reconcile driver on top of it, and one [`adapters`] module per
//! ecosystem behind the `ReleaseAdapter` trait. Stubs at founding; land in the
//! `release-engine` unit.

pub mod adapters;
pub mod bump;
pub mod bump_exec;
pub mod coordinator;
pub mod journal;
pub mod plan;
pub mod plan_store;
pub mod reconcile;
pub mod resume;
pub mod target_id;

pub use target_id::journal_target_ids;
