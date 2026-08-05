---
created: 2026-08-05
updated: 2026-08-05
type: bug
status: in-progress
priority: normal
epic: ossctl-phase4-build
commits:
- hash: 6cff876
  summary: cargo adapter one-target-one-publish + ADR-0004
---

# cargo adapter: closure-per-target double-publish race with multiple crates.io targets

## Description

Surfaced by the /llm-review of `release-cut-multi-target-ecosystem` (all four reviewers, independently).

## Problem
Now that `release cut` supports >1 target per ecosystem, ossctl's own contract declares two crates.io targets: `ossctl-core` then `ossctl`. The cargo adapter's `publish()` (crates/ossctl-core/src/release/adapters/cargo.rs) computes the transitive dependency CLOSURE rooted at each target's package and publishes every unpublished member it finds:

- Target 1 (`ossctl-core`): closure = [ossctl-core]. Publishes it. Because its own closure has no later dependent, `has_later_dependent` is false, so it does NOT `wait_for_index`.
- Target 2 (`ossctl`): closure = [ossctl-core, ossctl]. It re-encounters `ossctl-core` and only skips it if `is_published` sees it on the crates.io index. During the normal publish→index lag (seconds–minutes), `is_published` returns false and the adapter runs `cargo publish -p ossctl-core` a SECOND time. crates.io rejects the duplicate upload → the publish phase fails with a partial publish (ossctl-core landed, ossctl not).

Two authorities compute overlapping publish orderings (the coordinator orders targets; the adapter re-orders inside each target's closure), and the boundary between them is exactly where index lag hides. `is_published`'s registry-error-as-"not published" also treats a registry outage as permission to attempt a (duplicate) publish, inconsistent with the resume layer's tri-state discipline.

Related gap (same root): a cargo target that publishes several crates writes only ONE journal receipt (for the root package), so `resume`/`verify` lose track of which closure members landed — contradicting the coordinator's 'journal precisely what landed' guarantee.

## Why it was not fixed in release-cut-multi-target-ecosystem
That issue scoped to the plan/journal-id/coordinator layer (per-target journal ids + dep-ordered cut) and is landed & green. Fixing this properly means changing the cargo ADAPTER's publish model — the reviewers' preferred fix is 'one plan target = one publish unit: publish only t.package, wait for that package's own workspace deps to be index-visible first, remove the closure/topo logic from publish()'. But that would BREAK the existing single-target-multi-crate-workspace use case (a contract with ONE `ossctl` rust target expecting the adapter to publish the whole workspace closure), which the closure logic was built for and has tests for. Choosing between the two target models (every publishable crate is its own declared target, vs. one target owns the workspace closure) is a maintainer decision.

## Options (from the review)
1. One plan target = one publish unit. Adapter publishes only t.package and waits for its intra-workspace deps' index visibility before publishing. Coordinator owns cross-target ordering. (Breaks single-target-multi-crate unless every crate is declared a target.)
2. Coordinator-level index-wait between same-ecosystem dependency-related targets (coordinator learns cross-target dep edges, e.g. via one `cargo metadata` up front).
3. Keep closures but compute all cargo targets' closures during preflight, assign each workspace member to exactly ONE target, and reject ambiguous overlap.

## Also relevant (adjacent, may fold in or split)
- `to_journal_receipt` drops the adapter/registry identity; `reconcile`/`resume` re-derive the verify path from ecosystem's default adapter. With several rust targets on different channels (crates.io / gh-releases / homebrew, all ecosystem=`rust`), a homebrew/gh-releases receipt would be verified as a crates.io lookup and could misclassify as Missing → blocked resume. Currently moot because cargo-dist publish is Unsupported and homebrew-under-rust can't be cut through the engine yet (see release-engine-cut-cargo-dist-flow), but it becomes real once those land.

Issue: release-cut-multi-target-ecosystem
