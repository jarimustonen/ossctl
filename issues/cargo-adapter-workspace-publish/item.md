---
created: 2026-08-04
updated: 2026-08-04
type: bug
status: in-progress
priority: high
epic: ossctl-phase4-build
---

# cargo-publish adapter can't publish a multi-crate workspace (ordering + index wait)

## Description

Found during ossctl's OWN 0.1.0 self-cut (stint #9). The cargo-publish adapter (crates/ossctl-core/src/release/adapters/cargo.rs:111) runs 'cargo publish -p <t.package>' for the single named package only. For a workspace where the CLI crate depends on a sibling lib crate (ossctl depends on ossctl-core =0.1.0), this FAILS: crates.io rejects the CLI because its dependency isn't published yet ('no matching package named ossctl-core found'). 

The adapter must, for a workspace: (a) discover the intra-workspace publish dependency order (topological), (b) publish dependency crates first, (c) WAIT for each to appear on the crates.io index before publishing dependents (cargo's own 'waiting for X to be available' behavior). Currently none of this exists. WORKAROUND USED for the real cut: published ossctl-core, waited for index, then ossctl — all by hand outside the engine. Also note cargo.rs:113 still carries a SKELETON marker for parsing the crates.io checksum. Relates to facts-workspace-members (workspace enumeration now exists in facts; the plan/coordinator should USE it to expand crates.io targets in dep order).
