---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@cargo-adapter-workspace-publish']
---

# Per-member publish receipts for multi-crate cargo cuts (verify/reconcile visibility)

## Description

Surfaced by the /llm-review + /assess-findings panel on `cargo-adapter-workspace-publish` (SPIN-OFF F7, 4/4 reviewer consensus).

The cargo adapter now publishes N workspace crates in dependency order under a single `rust` ecosystem target, but the coordinator journals exactly ONE `PublishReceipt` per ecosystem target — for `t.package` (the primary crate). Consequences:

- `release verify` / `release reconcile` / `release show` (wave-3) are blind to the non-primary members: a yanked or wrong-version dependency crate (e.g. `ossctl-core`) is never checked, so verify can report a clean release that is actually broken.
- The receipt's `remote_url` and version describe only the primary crate; member versions may differ.

NOTE: the concrete DANGER (a partial publish that leaves no journal record and wedges every resume with "already uploaded") is already fixed in `cargo-adapter-workspace-publish` by making the adapter's publish loop idempotent (probe-before-publish, skip already-landed members). This spin-off is about VISIBILITY/verification correctness, not resumability.

Proper fix reshapes load-bearing seam surfaces the parent issue deliberately kept untouched ("minimal, additive; do not reshape the seam"), so it needs its own design/ADR pass. Options to weigh:

1. Per-member receipts: extend `JournalReceipt` (and the adapter `PublishReceipt`) to carry `Vec<PublishedCrate { package, version, registry_url }>`; teach verify/reconcile to reconcile each.
2. Member-as-target: resolve workspace members during `release plan`, seal the dependency graph into `ReleasePlan`, and let the coordinator drive each crate as its own journalable target with index-visibility barriers between them (the reviewers' preferred structural fix).

Either way: capture the resolved member set + versions at PLAN time (in the plan hash) so the executed set is sealed, and add a coordinator-level test proving exactly what is journaled after member 1 lands and member 2 fails.

Context: `crates/ossctl-core/src/release/adapters/cargo.rs` (publish loop), `crates/ossctl-core/src/release/coordinator.rs` (`publish_phase`, `to_journal_receipt`), `crates/ossctl-core/src/protocol/{plan,journal,release}.rs`. Full triage: `history/assessment-cargo-adapter-workspace-publish.md`.
