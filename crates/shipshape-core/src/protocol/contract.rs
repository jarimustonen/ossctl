//! Public wire DTO for the normalized `OSS-RELEASE.md` contract (SCHEMA.md §4).
//!
//! This is the versioned public surface every `/shipshape-*` member reads. At founding
//! the wire DTO is exactly the canonical model — the serialized form of
//! [`crate::contract::schema::Contract`] *is* SCHEMA.md §4, so a second parallel
//! struct would be duplication with no divergence to justify it. This module
//! therefore re-exports the canonical types and owns the wire-version
//! declaration ([`CONTRACT_SCHEMA_VERSION`]).
//!
//! The seam the ADR mandates (internals refactor without a wire break) stays
//! open: when the internal model ever needs a shape the wire cannot follow,
//! introduce a dedicated `ContractDto` here plus a `From<&schema::Contract>`
//! conversion and pin it — exactly the "sanctioned escape hatch when it hurts"
//! posture ADR-0001 takes toward the domain-crate split. Until then, one model,
//! documented as the wire contract.
//!
//! Consumers read this document under the CLI's canonical `data` envelope:
//! `{schema_version, data: <this shape>, warnings}` — every `shipshape --json`
//! command shares that envelope, so the §4 fields live at `.data.*`, uniform
//! with `facts`, `audit`, and the rest.

pub use crate::contract::schema::{
    Adapter, Changelog, ChangelogMode, ChangelogSource, Contract, ContributionProvenance,
    DependencyBot, DocsSite, Ecosystem, HealthBadge, Maturity, ProvenanceLevel, Registry, Release,
    ReleaseLayout, ReleaseModel, Status, Target, VersioningBase,
};

/// The `OSS-RELEASE.md` contract-document schema version this build reads and
/// emits. Distinct from the JSON-envelope [`crate::SCHEMA_VERSION`]: this
/// versions the *contract document* (SCHEMA.md §4), that versions the *wire
/// envelope*. Both are `1` today. A breaking change to the §4 shape bumps this
/// (and [`crate::contract::schema::KNOWN_SCHEMA_VERSION`], its enforcement
/// twin), never silently (migration rule).
pub const CONTRACT_SCHEMA_VERSION: u32 = crate::contract::schema::KNOWN_SCHEMA_VERSION;
