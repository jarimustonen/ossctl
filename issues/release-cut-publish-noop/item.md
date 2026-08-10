---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: open
priority: high
---

# release cut real publish is a no-op: crate never reaches crates.io (index-visibility timeout on an unpublished crate)

## Description

## Symptom (real-world repro, downstream consumer)

Cutting **issuectl 0.8.1** (2026-08-10) — the first *real* `ossctl release cut`
in the `issuectl` repo — the publish phase failed and **nothing landed on
crates.io**, even though ossctl reported the earlier phases OK.

Sequence:
- `ossctl release plan --version 0.8.1` → sealed fine.
- `ossctl release cut --plan <id> --version 0.8.1` → phases `dry_run: ok →
  build: ok → publish: failed` with:
  > publish-phase failed on target `rust:issuectl`: `issuectl-core@0.8.1` was
  > not visible on the registry index within 300s; a crate that depends on it
  > cannot be published until it is.
- Polled crates.io for **9 minutes** afterward: `issuectl-core@0.8.1` stayed
  **HTTP 404** on the API and never appeared on the sparse index. So it was
  **never actually uploaded** — the 300s "index visibility" wait was waiting
  for something that was never published.
- `ossctl release verify <run>` reported **no publish receipt** for either
  target (`rust:issuectl`, `rust:issuectl-core`).
- **Manual `cargo publish -p issuectl-core` then `-p issuectl` worked
  instantly** (both crates live within seconds), confirming the crates
  themselves are healthy and the defect is in ossctl's publish step.

## Hypothesis

An index-visibility wait implies ossctl believed `cargo publish -p
issuectl-core` succeeded (exit 0) yet nothing reached crates.io. The signature
(cargo "success" + no upload + no receipt) is consistent with the real-cut
publish running as a **`--dry-run`/no-op** (or otherwise not actually
uploading). Because the downstream wiring
(`@wire-oss-release-as-release-path` in issuectl) only ever verified via
`ossctl release plan` — which is *all* dry-run — a real-publish defect was not
caught until the first real cut.

## Why this is distinct from existing issues (cross-refs)

- `@cargo-per-member-receipts` — about *recording* per-member receipts so
  verify/reconcile can see visibility. Related (verify showed no receipts here)
  but this issue is that the crate **was never uploaded at all**, not just that
  a receipt was unrecorded.
- `@publish-crates-yml` — dep-order idempotency of ossctl's *own* CI publish
  (`already exists` exit 101). Different failure mode (that is a duplicate
  publish; this is a *missing* publish).

Please dedupe/link as the maintainer sees fit — filing because the observed
"real cut publishes nothing" behavior did not seem covered.

## Adjacent friction observed in the same session (context, not necessarily this issue)

- **Version-bump semantics were surprising and cost a failed cut.** `ossctl
  release cut` publishes the version **already in `Cargo.toml`** — it does NOT
  bump the version or finalize the CHANGELOG — yet `--version X.Y.Z` on both
  `plan` and `cut` strongly implies ossctl manages the version. My first cut
  attempt published `0.8.0` (the un-bumped tree version) and failed on
  `already exists`. If cut is meant to publish the tree version, consider
  erroring when `--version` != the tree's `Cargo.toml` version (drift guard),
  or document the "bump in a `release:` commit before cut" contract loudly.

## Impact

The intended ossctl release path is unusable for real releases until the
publish step actually uploads. Downstream (issuectl) had to fall back to manual
`cargo publish`, and there is no CI crates.io path either (it was retired in
favor of ossctl). Tracked downstream as issuectl `@ossctl-cut-no-publish`.

## Acceptance Criteria

- [ ] Root cause found: real `release cut` publish does not upload (dry-run/no-op
      or equivalent).
- [ ] `release cut` performs a real `cargo publish` and records a receipt per
      member.
- [ ] Integration test asserts a cut makes the version actually appear on the
      (test/mock) registry — not just that cargo exited 0.
- [ ] `--version` drift guard or clear docs on the bump-before-cut contract.
