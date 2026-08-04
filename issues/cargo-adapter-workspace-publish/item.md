---
created: 2026-08-04
updated: 2026-08-04
type: bug
status: fixed
priority: high
epic: ossctl-phase4-build
commits:
- hash: ed5c65c
  summary: publish multi-crate workspaces in dep order with index-wait
- hash: f0df1b6
  summary: apply /llm-review + /assess-findings confirmed findings
closed: 2026-08-04
---

# cargo-publish adapter can't publish a multi-crate workspace (ordering + index wait)

## Description

Found during ossctl's OWN 0.1.0 self-cut (stint #9). The cargo-publish adapter (crates/ossctl-core/src/release/adapters/cargo.rs:111) runs 'cargo publish -p <t.package>' for the single named package only. For a workspace where the CLI crate depends on a sibling lib crate (ossctl depends on ossctl-core =0.1.0), this FAILS: crates.io rejects the CLI because its dependency isn't published yet ('no matching package named ossctl-core found'). 

The adapter must, for a workspace: (a) discover the intra-workspace publish dependency order (topological), (b) publish dependency crates first, (c) WAIT for each to appear on the crates.io index before publishing dependents (cargo's own 'waiting for X to be available' behavior). Currently none of this exists. WORKAROUND USED for the real cut: published ossctl-core, waited for index, then ossctl — all by hand outside the engine. Also note cargo.rs:113 still carries a SKELETON marker for parsing the crates.io checksum. Relates to facts-workspace-members (workspace enumeration now exists in facts; the plan/coordinator should USE it to expand crates.io targets in dep order).

## Outcome (fixed)

The `cargo-publish` adapter's `publish()`/`dry_run()` are now workspace-aware, entirely within the per-adapter file (`crates/ossctl-core/src/release/adapters/cargo.rs`); the coordinator seam was left intact.

**How it works:** `publish()` runs read-only `cargo metadata --no-deps --format-version 1`, keeps the members publishable to crates.io (drops `publish = false` and crates restricted to another registry), restricts the set to the transitive workspace-dependency **closure rooted at the target `package`** (so a plan approving one package publishes exactly that crate plus the workspace crates it needs — never an unrelated publishable crate), topologically orders them (alphabetical tie-break, cycle → error), and publishes each `cargo publish -p <member>` in order. After a member that a later member depends on, it **waits for crates.io to index** that version by polling the injected `RegistryQuery`, bounded by a 300s/crate timeout (`AdapterError::IndexTimeout`), sleeping via the new `Clock::sleep` (real thread-sleep in prod; test fakes advance a virtual clock). Each `cargo publish` is preceded by an idempotency probe so a resumed cut skips members already on the index instead of hard-failing on a duplicate upload. A single-crate workspace resolves to one `cargo publish` with no wait (regression-preserving). `dry_run()` reports the full ordered plan + per-dependency index-wait notes.

**ossctl's own 2-crate cut end-to-end via the engine:** YES — a `rust` target with `package = ossctl` now resolves the closure `{ossctl-core, ossctl}`, publishes `ossctl-core`, waits for its index visibility, then publishes `ossctl`. Idempotent probe means a re-run after a partial failure resumes cleanly rather than wedging.

**Wire shape:** unchanged (`PublishReceipt`/`DryRunReport` shapes preserved) → no `schema_version` bump.

**Review:** `/llm-review` (4-model panel) + `/assess-findings` → 6 FIX applied (idempotent publish, closure-scope + membership validation, hard-error on empty metadata, crates.io registry filter, dependency-accurate index-wait, mechanical nits), 1 SPIN-OFF, 3 DROP. Triage: `history/assessment-cargo-adapter-workspace-publish.md`. Full green gate passes.

**Known limitation / follow-up:** the coordinator journals ONE receipt per ecosystem target (the primary crate), so wave-3 `verify`/`reconcile` cannot see non-primary members. The concrete danger (non-resumable wedge) is removed by the idempotent publish; the visibility gap is spun off to `cargo-per-member-receipts` (reshapes the sealed plan + journal model, out of scope here). The `build()` `.crate`-checksum SKELETON marker and receipt `digest` remain as-is (out of scope; tracked under adapter-publish-completeness).
