//! The `OSS-RELEASE.md` release contract: schema, normalizer, and SPDX check
//! (port of `check-oss-release.py` — ADR-0001 §3).
//!
//! This is the **inter-skill contract**: every `/oss-*` member reads
//! `OSS-RELEASE.md` only through here, and `contract show` / `contract validate`
//! are the two presentation modes over the one normalization function. Stub at
//! founding; the normalizer lands in the `contract-command` unit.

pub mod normalize;
pub mod schema;
pub mod spdx;

pub use normalize::{normalize, normalize_str, LoadError, Normalized, Problems, CONTRACT_FILENAME};
pub use spdx::spdx_valid;
