---
created: 2026-08-20
updated: 2026-08-20
type: bug
status: untriaged
priority: normal
provenance: agent:issuectl-stint-wrapup
---

# release verify infers delivery from the destination, not the delegated run: pending and failed are indistinguishable

## Description

## Problem

`ossctl release verify` decides whether a delegated target was delivered by looking at
the **destination** (does the GitHub Release have assets?) rather than at the **delegated
run** that produces it. Three very different states therefore look identical:

- the delegated CI is still building → zero assets
- the delegated CI finished successfully → assets appear (correct)
- the delegated CI died (cancelled / failed) → zero assets

Because "not yet" and "failed for good" are indistinguishable, `verify` fails with a
generic `is missing at its destination` and no cause, and the operator cannot tell whether
to wait or to intervene.

## Observed (issuectl 0.16.0 cut, ossctl 0.9.0, run `01M0CCMR9FCZ8GN9HTDC6FGCSR`)

`dry-run`, `build`, `publish`, `tag` and `dist` all green. Both crates on crates.io, tag
pushed, GitHub Release correctly delegated to cargo-dist. Then:

```
→ verify
  verified: rust:issuectl-core:crates.io (matches)
  verified: rust:issuectl:crates.io (matches)
  verified: rust:issuectl:gh-releases (missing)
✗ verify failed
```

The real cause was **not** visible anywhere in that output: cargo-dist's workflow had been
**cancelled**, because its `aarch64-unknown-linux-musl` build job queued on a GitHub-hosted
runner for six hours (06:56 → 12:56) and hit GitHub's hard job ceiling. That cancellation
skipped every downstream job (`build-global-artifacts`, `host`,
`publish-homebrew-formula`, `announce`), so no Release was created and the Homebrew tap
stayed on the previous version, while the crates.io publish had already happened
irreversibly.

The operator's first reading of `(missing)` was "verify raced the CI" — a wrong and
plausible conclusion that cost a round-trip. Only a manual `gh run view <id> --json jobs`
revealed the cancellation. `gh run rerun <id> --failed` then completed in ~5 minutes.

## Expected

`verify` observes the state of the delegated run, not only its destination:

- delegated run `in_progress` → report **pending**, not missing
- delegated run `success` → check the destination as today
- delegated run `cancelled` / `failure` → report **failed with the cause** (which job,
  which conclusion), immediately, without waiting

## Proposal

1. For any target whose adapter delegates to CI (`cargo-dist`, and any future delegating
   adapter), resolve the delegated workflow run and read its conclusion and per-job
   breakdown. Surface the failing job in the error message.
2. Give **pending** its own exit code, distinct from **missing**. A caller must be able to
   branch on "not finished yet" versus "genuinely absent" without parsing prose.
3. A bounded wait/retry for `pending` is a reasonable addition, but it is **secondary**: on
   its own it does not fix the core defect, because a cancelled run would still be
   indistinguishable from a slow one until the timeout expires.

## Related

Same false-signal family as `cut-runs-own` (0.15.0's false-red), relocated from the `dist`
phase to `verify`. Fixing `cut-runs-own` contract-side removed the earlier cause and
exposed this one behind it.
