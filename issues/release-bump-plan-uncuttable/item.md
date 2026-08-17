---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: high
lane: release-safety
lane_seq: 2
---

# release plan --bump seals an un-cuttable plan, and the staleness error steers the operator into republishing the current version

## Description

## Summary

`ossctl release plan --bump <level>` seals a plan that `ossctl release cut` **always** rejects
as stale. Worse, the rejection names a `current_plan_id` that is the **no-bump** plan — so an
operator who follows the error's own guidance attempts to **republish the version already on the
registry** instead of cutting the intended new one.

Found while cutting `issuectl` 0.14.1 with ossctl **0.6.1** (commit `2846d66`).

## Reproduction

Repo `issuectl` at HEAD `068df55`, manifest version `0.14.0`, tree clean, `0.14.0` already
published to crates.io.

    $ ossctl release plan --bump patch --json
    plan_id: cb443c1e69d3a5fb42ccc99e8e32af414ea2d6f950be19fbdc7ca2c6792d70c0
    version: 0.14.1          # correct — the bump was understood
    phases:  [bump, dry-run-all, build-all, publish-all, tag, dist]
    bump:    {level: patch, from_version: 0.14.0, to_version: 0.14.1, changelog_finalize: true}

    $ ossctl release cut --plan cb443c1e69d3a5fb42ccc99e8e32af414ea2d6f950be19fbdc7ca2c6792d70c0
    {"error":{"code":"plan_stale","message":"the approved plan is stale: the current repository
     (HEAD 068df5527673, version 0.14.0) hashes to a different plan_id, so a commit, contract
     edit, or version change occurred since `release plan` — re-run `ossctl release plan` and
     approve the new plan_id before cutting",
     "expected":{"current_plan_id":"fc7ac8eceef204f1e406c73d58fe8bc781bfd5648be449020813d23a5c5890fd"}}}

**Nothing had changed between the two commands** — same HEAD, same clean tree, no contract edit.
Re-running `release plan --bump patch` returns the *same* `cb443c…` every time, and the cut
rejects it every time. Verified twice.

## Diagnosis

The staleness check recomputes the plan hash from repository state **without applying the bump**.
A `--bump` plan is keyed on the post-bump version (`0.14.1`) while the check computes the
pre-bump hash (`0.14.0`), so the two can never agree. `--bump` is therefore unusable: no plan it
seals can ever be cut.

## The dangerous part

The error tells the operator to use `current_plan_id: fc7ac8…`. That id is the **no-bump** plan —
"publish the version currently in the manifest", i.e. the version **already on the registry**.
I followed that guidance and the cut ran:

    → dry_run   ✓  (dry-run ok on both crates — did NOT catch it)
    → build     ✓
    → publish   ✗  refusing to skip the publish of `issuectl-core@0.14.0`: it is already on the
                   registry, but the crate published there (sha256 c0b0bac4…) is NOT
                   byte-identical to the artifact this cut would upload (sha256 ae5c49b0…)

Two things worth noting:

1. **`dry-run-all` did not catch it.** Both crates passed dry-run at a version that was already
   published; only the publish phase's byte-identity guard stopped it.
2. **The byte-identity guard is the only thing that prevented a bad outcome.** It refused to skip
   rather than record a receipt for a crate it did not publish. That guard is excellent and
   should be kept exactly as it is — but it should not be the *first* line of defence against
   ossctl's own error message.

Nothing was published and no tag was created; the run was abandoned cleanly and
`release verify` confirmed zero publish receipts. So the failure mode was safe **in this
instance**, purely because of the guard.

## Expected

1. `release cut` accepts the plan that `release plan --bump` sealed — the staleness check must
   compare against the plan's own bump-aware hash, not a pre-bump recomputation.
2. Failing that, `release plan --bump` should refuse to seal a plan it knows cannot be cut,
   rather than returning a plausible-looking `plan_id`.
3. The `plan_stale` error must never suggest a `current_plan_id` whose meaning is "republish the
   version already on the registry". If the recomputed plan would target an
   already-published version, say so explicitly instead of offering it as the remedy.
4. Consider having `dry-run-all` fail when a target version already exists on the registry —
   that is the phase whose job is to catch this before any upload is attempted.

## Workaround

Do the bump by hand (edit `version` in the workspace manifest, `cargo update --workspace`,
finalize the CHANGELOG, commit as `release: X.Y.Z`), then run `ossctl release plan` **without**
`--bump` and cut the id it returns. That path works: `issuectl` 0.14.1 was cut this way, all six
phases green.

## Comments

### 2026-08-17T05:47:46Z · @claude

Laned release-safety, second behind @release-tag-preempts-cargo-dist. Ordering rationale: this bug fails LOUDLY (the sealed plan is simply never cuttable, and the byte-identity guard stopped the dangerous follow-on before any upload), and it has a working documented workaround — the manual bump. The tag/cargo-dist collision fails SILENTLY, reporting a complete release while dropping the binaries and the Homebrew formula, so it goes first.

Context worth recording: stint #20 deliberately did NOT dogfood --bump on ossctl's own irreversible cut, decoupling its live acceptance to a downstream cut precisely because it was unproven. That call was correct — this report is that live acceptance, and it failed. --bump has never worked end-to-end.

The most alarming part is not the staleness bug but the remedy the error offers: pointing the operator at the no-bump plan means 'republish the version already on the registry'. Point 3 of the Expected list (never suggest a current_plan_id that targets an already-published version) should be treated as part of this fix, not a nice-to-have — an error message that leads a careful operator toward a bad publish is worse than the bug it reports.

### 2026-08-17T06:10:29Z · @claude

MECHANISM CORRECTED (2026-08-17, controlled repro in a scratch repo — orchestrator session, stint #22). The staleness hash is NOT broken: 'release cut --plan <bump-plan-id> --bump patch' ACCEPTS the sealed --bump plan (verified: the cut passed the plan_stale gate and entered the bump phase). CutArgs has a --bump flag documented as 'the same --bump <level> passed to release plan'. The field failure's mechanism is operational: the operator must repeat --bump at cut time, nothing in the plan output or the error says so, and the plan_stale error offers the no-bump plan id (= republish the already-published version). So the Diagnosis section above is wrong about the hash; the Expected list's items 2-3 (never seal an uncuttable plan / never steer to a republishing plan id) remain the real fix, plus discoverability. The decided fix is the durable plan store: see ../release-tag-preempts-cargo-dist/design.md (D1) — cut reads the bump disposition from the stored sealed plan, making the flag an optional must-match; the plan_stale error is rewritten to diagnose the actual difference and never present another plan id as remedy. Same repro also found a NEW bug: bump_exec requires a [workspace.package] version line, so a single-crate repo ([package] version) cannot --bump at all — filed as @bump-single-crate-manifest.

