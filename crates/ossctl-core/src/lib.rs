//! Core library for `ossctl` — the deterministic engine behind the `/oss-*`
//! Claude Code skill family.
//!
//! All logic that must be exact and identical for every caller lives here so it
//! is unit-testable without spawning the binary. The `ossctl-cli` crate is a
//! thin clap + I/O shell over this library (ADR-0001 §2).
//!
//! The founding architecture is fixed by the accepted ADRs under `docs/adr/`;
//! this crate realizes their module layout. At founding these are **module
//! stubs** — the compiling shape, no domain logic yet:
//!
//! - [`contract`] — `OSS-RELEASE.md` schema + normalizer (port of
//!   `check-oss-release.py`); [`contract::schema`] is the ONE canonical serde
//!   model (ADR-0003).
//! - [`facts`] — deterministic repo-fact detector (port of
//!   `infer-repo-facts.py`), behind the repo ports.
//! - [`audit`] — readiness scoring over the normalized contract + facts.
//! - [`dist`] — the deterministic cargo-dist `dist-workspace.toml` generator
//!   (renders the binary-release infra from the contract's `distribution` block).
//! - [`release`] — the resumable per-ecosystem release-cut engine (ADR-0002):
//!   sealed plan, phase-barrier coordinator, event-sourced journal, reconcile.
//! - [`protocol`] — the versioned public JSON/JSONL envelopes + DTOs (§10/§12),
//!   versioned independently of the internal domain types.
//! - [`ports`] — the injected effect seam (`CommandRunner`, `Clock`, `IdGen`,
//!   `RegistryQuery`, `Fs`, `GitRepo`) so each domain is testable without
//!   touching the real filesystem, git, network, or clock.
//!
//! `ossctl-core` is the canonical library surface, so public items must carry
//! doc comments (`#![warn(missing_docs)]`); level policy otherwise lives in the
//! workspace `[workspace.lints]` table.
#![warn(missing_docs)]

pub mod audit;
pub mod contract;
pub mod dist;
pub mod facts;
pub mod ports;
pub mod protocol;
pub mod release;
pub(crate) mod vcs;

pub use protocol::{SCHEMA_VERSION, SUPPORTED_SCHEMAS};
