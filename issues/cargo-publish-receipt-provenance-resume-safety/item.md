---
created: 2026-08-05
updated: 2026-08-05
type: bug
status: open
priority: normal
epic: ossctl-phase4-build
---

## Description

Surfaced by the 4-model `/llm-review` of ADR-0004 (`cargo-adapter-multitarget-double-publish`). These are pre-existing `SKELETON` gaps in the cargo publish/receipt path that ADR-0004 did **not** close (it fixed the double-publish ordering). Clustered here because they share one root: the cargo receipt has no content identity, so the resume/reconcile path cannot prove provenance.

## Problems (from the review)

1. **Presence-only idempotency blesses an unrelated/conflicting crate.** `publish()`'s pre-publish probe `is_published(t.package, t.version)` treats "a version with these coordinates exists on crates.io" as proof *this cut* published the intended bytes, then synthesizes a receipt with no digest and a fresh timestamp. It silently accepts a version published by another actor, a stale earlier-attempt version, or a version whose contents don't match the current commit. `verify_via_registry`/`classify_receipt` then classify any matching version as `Matches` because the receipt carries no digest. (GPT-5.6, Opus-4.7)
2. **`Ok(false)` is not "definitively absent" after an ambiguous publish.** If `cargo publish` uploads successfully but the process dies before the receipt journals, a resume probing during the publish→index lag sees an empty version list and could run a duplicate irreversible publish. This is the same lag boundary as the original bug, moved from cross-target closure to resume reconciliation. Mitigation: journal a publish-*attempt* fact before shelling out, and only convert `Missing`→"publish" when no prior attempt is recorded. (GPT-5.6)
3. **No manifest-vs-plan version guard.** `cargo publish -p <pkg>` publishes whatever version the manifest holds; the adapter never checks that the target member's `cargo metadata` version equals `t.version`. A silently-failed bump ⇒ publishes the wrong version, records a false receipt/URL for the planned version. Add a version-equality guard before build/publish (needs a proper `AdapterError::VersionMismatch`, not `Command`-abuse). (GPT-5.6, DeepSeek)
4. **Digest capture is a skeleton but not optional for an irreversible adapter.** `make_receipt(.., None, ..)` — the `.crate` checksum is never parsed and journaled, so the ADR-0003 reconcile state table's `Conflicts` branch is undetectable. Production-safe cargo resume needs: build records the exact `.crate` path → local sha256 computed+journalled → crates.io per-version checksum queried (new `RegistryQuery` capability) → resumed/adopted publications require checksum equality. (GPT-5.6, Opus-4.7)
5. **Idempotent-path receipt timestamp drift** (minor): the early-return manufactures a receipt with `ctx.clock.now_unix()` (observation time), not the actual publish time. Distinguish "observed published at T" from "published at T", or document the limitation. (Opus-4.7)

## Scope note

Requires extending the `RegistryQuery` port (per-version checksum) and adding journal publish-attempt events + new `AdapterError` variants. Larger than the ADR-0004 fix; deliberately deferred. `release-engine-cut-cargo-dist-flow` and this together gate "cargo cut is production-safe end-to-end".

Refs-Issue: cargo-adapter-multitarget-double-publish

## Update 2026-08-06 — `RegistryQuery` version-existence arm is now wired (partial progress on #4)

`release-publish-registry-query-not-wired` (commits 8fc1e85/483ce0b) landed the
crates.io `RegistryQuery` for ecosystem `rust`: `published_versions` now returns the
real version list from the sparse index, so the **version-existence** half of point
#4's prerequisite chain exists (proven end-to-end against live crates.io —
`ossctl-core@0.2.0` → `matches`, `@99.0.0` → `missing`). Still open here:

- **Point #4 (checksum):** the port still returns only version *strings*, not the
  per-version `cksum`. `classify_receipt`'s `Conflicts` branch remains undetectable
  until a checksum-returning capability is added. (The sparse-index body the new arm
  already fetches *carries* `cksum` per line — a checksum arm can read it from the
  same response, no extra request.)
- **Point #2 (stale-negative after an ambiguous publish):** the new arm reads the
  sparse index, which lags a just-accepted upload. The `/llm-review` of the wiring
  re-raised exactly this: a resume probing during the publish→index window can see
  the version absent (`Ok(false)`) and retry an irreversible upload. The
  publish-*attempt* journaling fact proposed in #2 is still the fix; the wiring did
  not address it (out of scope).

Refs-Issue: release-publish-registry-query-not-wired
