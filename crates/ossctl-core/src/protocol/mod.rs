//! Versioned public protocol surface (`AGENTS-AI-FIRST-CLI.md` §10/§12).
//!
//! Every `--json` payload and every `--output=jsonl` event carries a
//! `schema_version`. This module owns that contract and the public
//! JSON/JSONL DTOs that ride on it. DTOs are versioned **independently** of the
//! internal domain types (`contract::schema`, `facts`, `audit`, `release`) so
//! `ossctl-core` can refactor internals without a wire break (ADR-0001 §2).
//!
//! At founding this holds only the envelope schema version; the concrete DTOs
//! (contract document, facts report, audit gap-report, release events) land
//! with their owning units and each becomes a hot file under the migration rule
//! (bump `SCHEMA_VERSION` on a breaking change, never silently).

/// Current envelope/DTO schema version for `ossctl`'s public JSON output.
///
/// Monotonic integer. Breaking changes (removing/renaming fields, changing
/// types or enum semantics, tightening nullability, changing event ordering)
/// bump this; additive changes (new optional fields) do not.
pub const SCHEMA_VERSION: u32 = 1;

/// The set of envelope schema versions this binary can emit and understand.
///
/// Surfaced by `ossctl version --json` so a caller can detect drift between its
/// trained expectations and the running binary (`AGENTS-AI-FIRST-CLI.md` §10).
pub const SUPPORTED_SCHEMAS: &[u32] = &[SCHEMA_VERSION];
