//! The sealed, content-addressed release plan — the read-only pre-image the
//! human approves (ADR-0002).
//!
//! `release plan` computes and seals a `plan_id`; `release cut --plan <plan_id>`
//! executes it and refuses on repo drift. The binary never prompts: it plans
//! and exits at the approval boundary. Stub at founding; lands in the
//! `release-engine` unit.
