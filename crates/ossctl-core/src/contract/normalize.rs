//! Normalization pipeline: validate every field/enum/cross-field floor,
//! materialize all defaults, and expand `targets` from `ecosystems`.
//!
//! `contract show` returns the canonical (normalized) form; `contract validate`
//! runs the identical pipeline and discards the document, emitting only
//! pass/fail (ADR-0001 §1). Stub at founding; lands in the `contract-command`
//! unit.
