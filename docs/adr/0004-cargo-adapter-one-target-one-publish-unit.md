# 0004 — cargo adapter: one plan target = one publish unit

**Status:** Accepted
**Date:** 2026-08-05
**Authors:** Jari Mustonen (decision owner); `cargo-adapter-multitarget-double-publish` spinoff worktree (agent). Problem surfaced independently by all four reviewers of a `/llm-review` of `release-cut-multi-target-ecosystem`.

> Companion to **ADR-0002** (release engine: adapter model + phase-barrier coordinator). This ADR settles who owns publish ordering for the rust ecosystem once a contract declares **more than one crates.io target**, and removes a double-publish / partial-publish trap the original cargo adapter carried.

---

## Context

ADR-0002 fixed the phase-barrier coordinator (dry-run-all → build-all → publish-all → tag-once, coordinator-only tagging) and made each adapter operate on its own `AdapterTarget` slice. `release-cut-multi-target-ecosystem` then added support for **more than one target per ecosystem**, cut in dependency order — so `ossctl`'s own contract declares two crates.io targets: `ossctl-core`, then `ossctl`.

The cargo adapter's `publish()`, however, did not publish a single crate. To support a workspace whose crates depend on one another (crates.io rejects a crate whose sibling dependency is not yet indexed), it computed — per target — the transitive workspace-dependency **closure rooted at that target's package** and published every unpublished member of the closure in its own topological order, index-waiting between dependent members.

With two crates.io targets this produces **two authorities computing overlapping publish orderings**:

- The **coordinator** orders the targets (`ossctl-core` before `ossctl`).
- The **adapter** re-orders *inside* each target's closure.

The boundary between them is exactly where the crates.io **publish→index lag** hides:

- Target 1 (`ossctl-core`): closure `[ossctl-core]`. Publishes it. No later dependent in its own closure ⇒ no index-wait.
- Target 2 (`ossctl`): closure `[ossctl-core, ossctl]`. It re-encounters `ossctl-core` and only skips it if `is_published` sees it on the index. During the normal seconds-to-minutes publish→index lag, `is_published` returns `false`, so the adapter runs `cargo publish -p ossctl-core` **a second time**. crates.io rejects the duplicate upload → the publish phase fails with a partial publish (`ossctl-core` landed, `ossctl` did not). crates.io publishes are irreversible (yank-only), so this is the most dangerous failure class in the engine.

Two adjacent defects share the same root:

- **`is_published` treated a registry lookup *error* as "not published"** (proceed and let cargo be the authority) — inconsistent with the reconcile layer's tri-state discipline (ADR-0003 §4), where an outage is `Unknown` and **never** `Missing`. Under an outage the closure model would *attempt* a duplicate publish.
- **One receipt for the closure's root only.** A cargo target that publishes several crates wrote a single journal receipt, so `resume`/`verify` lost track of which closure members actually landed — contradicting ADR-0003's "journal precisely what landed".

---

## Options considered (from the review)

1. **One plan target = one publish unit.** The adapter publishes only its own target's package and waits for *that package's* intra-workspace dependencies to be index-visible first; the coordinator owns all cross-target ordering. Breaks the single-target-multi-crate-workspace case unless every publishable crate is declared as its own target.
2. **Coordinator-level index-wait between dependency-related same-ecosystem targets** — the coordinator learns cross-target dependency edges (e.g. one `cargo metadata` up front) and waits between them.
3. **Keep closures but disjointly partition them** — compute every cargo target's closure during preflight, assign each workspace member to exactly one target, and reject ambiguous overlap.

---

## Decision

**Adopt Option 1: one plan target = one publish unit.**

### 1. The adapter publishes only its own target's package

`CargoAdapter::publish()` runs exactly one `cargo publish -p <t.package>`. The closure/topological-walk logic (`dep_closure` / `topo_sort` / `has_later_dependent`) is removed. There are no longer two authorities: the coordinator alone orders targets, and it already cuts same-ecosystem targets in dependency order (`release-cut-multi-target-ecosystem`) — a dependency's target before its dependents'.

### 2. Wait on *this package's own* deps, not on later dependents

Before publishing its package, the adapter discovers that package's publishable intra-workspace dependencies (read-only `cargo metadata`) and waits for each to be crates.io-index-visible. Each dependency has its own target, cut earlier by the coordinator, so this only closes the publish→index-lag window between two coordinator-ordered publishes — never re-publishes another target's crate. A crate with no publishable workspace dependencies publishes immediately with no wait.

### 3. The ADR-0002 phase barrier is preserved

Dry-run-all → build-all → publish-all → tag-once, coordinator-only tagging, unchanged. `dry_run` reports one `cargo publish … --dry-run` for the target's package plus a note listing the workspace dependencies a real cut would wait to index first.

### 4. `is_published` is tri-state and fails closed

The pre-publish idempotency probe now distinguishes **published** (`Ok(true)`, skip), **definitively absent** (`Ok(false)`, safe to publish), and **registry-unreachable** (`Err(AdapterError::RegistryUnavailable)`). A registry error is **never** read as "not published"; the publish fails closed rather than risk a duplicate, irreversible upload — mirroring the reconcile layer's outage ⇒ `Unknown` ⇒ never-`Missing` discipline (ADR-0003 §4).

### 5. One target = one crate = one receipt

Because each target now publishes exactly one crate, each writes exactly one journal receipt. `resume`/`verify` track every published crate precisely — the "one receipt for the root package only" gap is closed structurally rather than by discipline.

### 6. Consequent target model: each publishable crate is its own declared target

The workspace publish model becomes **"each publishable crate is its own declared target"**, which is already what `/oss-init` emits. A multi-crate workspace that wants every crate on crates.io declares every crate as a target. A target whose package depends on a workspace crate that is *not* itself a declared target will **time out** waiting for that crate to appear on the index — the honest signal that it must be declared, rather than a silent whole-workspace publish.

---

## Consequences

**Positive**

- The double-publish / partial-publish trap is gone: with a single authority for ordering, no code path re-publishes a crate another target already published. Regression-tested at both the adapter level (a dependent target under index lag publishes only its own crate) and the coordinator level (two dep-ordered targets under a multi-poll index lag publish each crate exactly once).
- The publish path is **fail-closed under a registry outage** — an unreachable registry aborts rather than attempting a duplicate irreversible upload.
- `resume`/`verify` reason over one receipt per crate, matching ADR-0003's journal guarantee.
- The adapter is simpler: no per-target closure, topo-sort, or cycle detection — the coordinator's dependency order is the only order.

**Costs / risks accepted**

- **Breaking change to the single-target-multi-crate-workspace model.** A contract that declared one `rust` target and expected the adapter to publish the whole workspace closure no longer does. Accepted: `/oss-init` already emits one target per publishable crate, and the alternative (two authorities across the index-lag boundary) is the very bug being fixed. The failure mode for a mis-declared workspace is a clear index-wait timeout, not a silent partial publish.
- **A dependency published by a target the coordinator has not yet cut will time out** rather than being pulled in implicitly. Accepted as the honest signal that the crate must be declared as its own target (and ordered by the coordinator).

**Rejected alternatives**

- **Option 2 (coordinator-level cross-target index-wait).** Rejected: it keeps ordering knowledge split between the coordinator and `cargo metadata`, and pushes rust-specific workspace-dependency reasoning up into the ecosystem-agnostic coordinator — the opposite of ADR-0002's per-target data-hiding.
- **Option 3 (disjoint closure partition).** Rejected: it retains the closure machinery and its two-authority ordering, adding a preflight overlap-rejection pass to paper over the same seam rather than removing it. One-target-one-publish deletes the seam entirely.
- **Leaving `is_published` erroring-as-absent.** Rejected: it is the exact inconsistency with ADR-0003's tri-state reconcile discipline that lets an outage authorize a duplicate irreversible publish.

---

Refs-Issue: cargo-adapter-multitarget-double-publish
