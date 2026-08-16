---
created: 2026-08-05
updated: 2026-08-16
type: feature
status: duplicate
priority: normal
epic: ossctl-phase4-build
related: ['@cargo-publish-receipt-provenance-resume-safety']
closed: 2026-08-16
---

## Description

Surfaced by the `/llm-review` of ADR-0004 (`cargo-adapter-multitarget-double-publish`), flagged by all four reviewers. ADR-0004 accepted this as a documented cost; this issue is the follow-up that removes it.

## Problem

Under the ADR-0004 model (each publishable crate is its own declared target, coordinator cuts dep-first), an **under-declared** plan is only detected at publish time: a target whose package depends on a workspace crate that is not a declared target sits in `wait_for_index` for up to `INDEX_WAIT_TIMEOUT_SECS` (300s) *per missing dep*, then fails with `IndexTimeout` — during the irreversible publish phase, after other crates may already have landed. The message (now improved) still can't say "declare it as a target" with certainty because the adapter, by design (`AdapterTarget` hides the rest of the plan), doesn't know the declared-target set.

Two related gaps the reviewers noted:
- The coordinator/normalizer is asserted to own dependency ordering, but the multi-target coordinator tests only pass **already dep-ordered** plans — no test proves a reversed input is reordered/rejected. A reversed plan would currently wait-then-timeout, not fail fast.
- A dependency whose required version is *already* on crates.io makes the wait clear immediately, so an under-declared plan can even **silently succeed** without the dep being a target.

## Fix (plan-time, ecosystem-neutral coordinator)

Resolve the cargo workspace dependency graph during plan construction / `validate_plan` preflight (one `cargo metadata`), and reject before any external action when: a target's publishable workspace dep is not itself a declared target; targets are not in dependency order (or reorder them); or a dep target uses the wrong registry/adapter or an incompatible version. Persist explicit dep edges / a sealed topo order in the plan so the generic coordinator executes an already-validated order and the adapter does only the immediate index-readiness check. Keep Rust-specific graph logic in a cargo-aware planner, not in the coordinator core. Add tests: reversed-input reorder/reject; missing-dep-target rejection; wrong-registry/adapter dep rejection.

Refs-Issue: cargo-adapter-multitarget-double-publish

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Plan-time cargo target coverage belongs with the retained cargo release hardening cluster, not as a separate backlog item.
