//! Deterministic repo-fact detector (port of `infer-repo-facts.py` —
//! ADR-0001 §3).
//!
//! A pure function of `(repo, HEAD)`: detects ecosystems, packages, CI presence,
//! tags, and maturity signals, behind the [`crate::ports::GitRepo`] /
//! [`crate::ports::CommandRunner`] ports. Feeds `ossctl facts` and the audit
//! scorer. Stub at founding; lands in the `facts-command` unit.
