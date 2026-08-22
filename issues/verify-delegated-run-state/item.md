---
created: 2026-08-20
updated: 2026-08-22
type: bug
status: in-progress
priority: high
provenance: agent:issuectl-stint-wrapup
lane: verify-observers
lane_seq: 10
collision: [crates/ossctl-core/src/release/coordinator.rs]
commits:
- hash: cc6491b7f24560b80373b98a6fc6905727555da6
  summary: 'fix(release): observe delegated workflow runs'
- hash: c0146ac27ed4620dc7db1bf876012a8655e6acfa
  summary: 'chore(issue): start delegated run verification fix'
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

## Comments

### 2026-08-20T16:19:30Z · @agent-stint-24

Triaged into verify-seam/25 (stint #24). HIGH, collision on release/distribution.rs.

Placed BEFORE delegated-verify-window-ux (30) deliberately: window-ux rebuilds the polling loop (one barrier deadline, progress events), and that loop is much easier to build correctly on top of a verify that can already distinguish pending from failed. Doing them in the other order would mean writing the progress UX twice.

Kept per the repo issue standard: the failure is reachable here (observed on a real issuectl 0.16.0 cut), it follows an irreversible crates.io publish, and it contradicts ADR-0002's verify guarantee that a target is observed at its destination with an actionable verdict. The damage is not just an error message: the operator's plausible first reading of '(missing)' was 'verify raced CI', which was wrong and cost a round-trip while a cancelled cargo-dist run sat undiscovered.

Reopen/close condition: close as fixed when a cancelled or failed delegated CI run makes verify report the cause (which job, which conclusion) immediately rather than waiting out the window, and when 'pending' is distinguishable from 'missing' without parsing prose. Close as wontfix only if resolving the delegated run proves unreachable for a non-GitHub delegating adapter, recording which adapter defeated it.
