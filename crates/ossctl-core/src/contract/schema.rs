//! The ONE canonical serde model for `OSS-RELEASE.md` (ADR-0003).
//!
//! These types are the single normalization model that `contract show`,
//! `contract validate`, `audit`, the facts consumers, and release planning all
//! use — no second parser anywhere. Public JSON DTOs (`protocol`) are versioned
//! independently so internals can change without a wire break.
//!
//! Hot file (ADR-0001): a change here ripples to every family member. Empty at
//! founding; the serde types land with the `contract-command` unit.
