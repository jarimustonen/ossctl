# Design: observable cuts + a durable plan store

**Scope.** One design for the open `release-safety` cluster, written 2026-08-17 (stint #22
orchestrator). Covers and is referenced by: `release-tag-preempts-cargo-dist`,
`release-bump-plan-uncuttable`, `resume-drift-after-fix`, `release-verify-homebrew-tap`,
`release-verify-delegated-github-release`, and (partially) `release-abandon-break-stale-lock`
and `intake-feature-ossctl-73e870268475`. The theme (TODO.md handoff): **the engine reports
success without verifying the artifact**, plus two staleness-seam defects. Root-cause analysis
and field evidence live in the issue bodies; this doc fixes the architecture decisions and the
work-unit briefs.

**New evidence (2026-08-17, this session).** A controlled repro (scratch repo, uncompilable
crate so the cut stops at dry-run) established that `release cut --plan <bump-plan-id> --bump
patch` **accepts** the sealed `--bump` plan — the staleness hash is NOT broken. The field
failure's mechanism is operational: the operator must repeat `--bump <level>` at cut time,
nothing tells them so, and the `plan_stale` error actively offers the no-bump plan id (= the
already-published version). The same experiment surfaced a new bug: `bump_exec` requires a
`[workspace.package] version` line, so a single-crate repo (`[package] version`) cannot use
`--bump` at all (filed as `bump-single-crate-manifest`).

---

## D1. Durable plan store; cut consumes the stored plan (ADR-0003 amendment)

`release plan` persists the sealed plan document (the exact canonical-JSON bytes that were
hashed) at `<git-common-dir>/ossctl/plans/<plan_id>.json`, alongside the existing
`releases/<run_id>/` journal tree. Content-addressed and immutable: writing is create-if-absent
(identical content by construction; never overwrite a differing file — that is an integrity
error).

`release cut --plan <id>`:

1. Loads `plans/<id>.json`. Integrity check: re-hash the stored document; it must equal the
   filename/plan id, else refuse (`plan_store_corrupt`).
2. Takes the bump disposition **from the stored plan** — the `--bump` CLI flag becomes an
   optional must-match confirmation (mismatch = user error naming both).
3. Drift check unchanged in spirit: re-derive the plan from the current tree using the stored
   plan's bump level; the recomputed id must equal `--plan`, else `plan_stale`.
4. **Fallback** when no stored plan exists (plan sealed by an older binary or on another
   machine): today's flag-driven derivation, plus the improved error below.

`plan_stale` error rewrite (all paths, fallback included):

- Diagnose the actual difference: HEAD moved (show both shas), manifest version changed (show
  both), or contract/facts changed. With a stored plan this is a field-by-field diff; without
  one, probe all three bump levels and, when one reproduces the given id, say exactly
  "this plan was sealed with `--bump <level>` — pass `--bump <level>` to `release cut`".
- **Never present another plan id as the remedy.** Drop `current_plan_id` from the message
  text; in the JSON envelope rename the field to `recomputed_plan_id` with an explicit
  warning note when its version equals the version already in the manifest (i.e. cutting it
  would attempt to republish). The remedy is always "re-run `ossctl release plan` (with
  `--bump` if intended) and approve what it prints".

## D2. Resume executes the stored plan — never the live tree

`release resume <run>` currently re-derives the plan from the **working tree** and refuses on
any drift, which makes a run permanently unresumable the moment the operator fixes the defect
that stopped it (`resume-drift-after-fix`). But since `release-cut-clean-checkout`, execution
never reads the live tree anyway — every effect runs from a checkout of the sealed commit.
The tree-drift check at resume time is a proxy with no remaining safety content.

Change: resume loads the run's sealed plan from the plan store (keyed by the journal's
`plan_id`), integrity-checks it, and executes it against the sealed-commit checkout exactly as
cut would. No working-tree comparison at all. The run's publishes stay internally consistent
(all from the sealed commit) regardless of where `main` has moved. This also gives `--bump`
runs a correct resume for free (the stored plan carries the bump). Fallback when the plan
file is absent: today's re-derivation + drift refusal, with the error additionally saying
that runs planned by ossctl ≥ this version resume across code fixes.

`release abandon` stays as-is (the escape for runs that cannot or should not finish).

## D3. Mandatory post-cut `verify` phase (ADR-0002 amendment): green means observed

New final coordinator barrier after `dist`: **verify-all**. The run flips to `Completed` only
when every target's artifact has been **observed** at its destination. "A publish target that
cannot be observed after the fact is not a publish target."

Per-target obligations (each via the existing effect seams — `RegistryQuery`/`http_get`,
`CommandRunner`; no new ambient I/O):

- **crates.io** (`cargo-publish`): index lookup — version present (+ digest match when the
  receipt carries one). Mostly a re-assertion of the existing self-visibility confirm.
- **homebrew** (`homebrew-tap`): fetch the tap's `Formula/<name>.rb` (raw content over the
  `http_get` seam), assert (a) ossctl ownership marker on line 1, (b) the formula version
  equals the plan version, (c) every per-platform URL/sha256 stanza present. This replaces the
  adapter's current `verify → Unknown` override — the tap IS observable; six silent-green
  releases happened because nothing looked.
- **gh-releases delegated to CI** (`cargo-dist`): poll `gh release view v<version> --json
  assets` through the runner until the Release exists and carries at least the per-platform
  archive set the plan declares, bounded (default ceiling 20 min, poll 15 s — CI builds a
  matrix). Timeout ⇒ the verify phase FAILS (journalled, resumable): a resumed run re-enters
  verify only.
- **engine-created GitHub Release** (`manual`/binary): Release exists; declared assets
  uploaded.

Journal: new `target_verified { target, outcome }` events + `phase_entered/completed verify`;
schema v4 → v5 (reducer keeps accepting v4 files; a v4 run completed at `dist` remains
`Completed`). `VerifyOutcome::Unknown` is **not green** in this phase: it fails the barrier
with an honest "could not observe" message. No `--allow-unverified` on `cut`; the flag remains
resume-only.

`release verify <run>` (the standalone command) gains the same real per-target checks, so
post-hoc verification of old runs works too, and TODO recipe steps 4–5 (manual `gh release
view` + tap `curl`) become automated.

## D4. Facts↔contract distribution cross-check: refuse the under-declared cut

Trigger case (issuectl 0.14.1): the repo really runs cargo-dist + a Homebrew tap, but the
contract declares only crates.io targets — the engine, seeing no gh-releases target, created
the GitHub Release itself, collided with cargo-dist's `host` job, and silently dropped the
binaries + formula while reporting green.

Detection (facts side): `distribution_surface` facts — presence of `dist-workspace.toml` /
`[workspace.metadata.dist]`, a tag-triggered release workflow (`on: push: tags:` in
`.github/workflows/*.yml`), and the contract's own `distribution:` block fields.

Enforcement:

- `contract validate` / `release plan`: WARN on each undeclared-surface finding ("repo has a
  cargo-dist release workflow but no `gh-releases` target — the tag phase would collide with
  it"; "distribution.homebrew_tap is set but there is no homebrew target — the tap leg would
  be silently skipped", the `intake-feature-ossctl-73e870268475` case).
- `release cut`: **hard refusal** (user error) on the same findings, before any run is
  created. No escape flag — the remediation is a one-line contract fix, and every known
  instance of proceeding anyway shipped a broken release.

This makes the D3 verify phase and D4 complementary: D4 blocks the mis-declared cut up front;
D3 catches everything else after the fact.

## D5. Stale-lock break in `release abandon` (narrowed per issue scope)

The lock file records holder identity at acquisition: `{pid, hostname, started_unix,
run_id?}` (JSON). On `WouldBlock`, `abandon` reads it; if the recorded host matches and the
pid is not alive (a `kill -0 <pid>` probe through the runner), it removes the stale lock,
retries once, and prints what it did. If the holder is alive, unreadable, or another host:
today's actionable error. Explicitly OUT (issue scope): pid-reuse defence, network-fs
correctness, advisory-lock migration.

## D6. End-to-end harness: the cut is testable without cutting

`crates/ossctl-cli/tests/e2e/` integration tests drive the **real compiled binary** against a
scaffolded temp git repo (contract + crate + commits), with external commands intercepted by
PATH-shim fakes (`cargo`, `gh`, `curl`, `sha256sum` — tiny scripts writing recorded args and
replaying canned outputs; `git` stays real). Assertions at the observable-surface level:
sealed plan → cut phases → journal contents → error envelopes.

Must-cover scenarios (each one is a bug this repo shipped because only production tested it):
plan `--bump` → cut acceptance (with and without the flag), the `plan_stale` envelope wording,
resume after a code fix (D2), the cargo-dist under-declared refusal (D4), verify-phase
red/green (D3), stale-lock break (D5). This does not replace "tuotanto on testi" — it front-
loads it.

---

## Work units and sequencing

Hot-file rules (AGENTS.md): `coordinator.rs`/`adapters/mod.rs` and shared protocol modules are
semantic — never two workers in parallel; `Cargo.toml`/dispatch/CATALOG rows are append-safe —
union-resolve.

| Unit | Design | Files (primary) | Wave |
|------|--------|-----------------|------|
| W-A `plan-store-cut-resume` | D1+D2 | `release/plan.rs`, new `release/plan_store.rs`, `cli/release.rs`, `journal.rs` (paths only) | 1 |
| W-B `distribution-cross-check` | D4 | `facts/`, `contract/` validate, small cut preflight hook | 1 |
| W-C `abandon-stale-lock` | D5 | `journal.rs` (lock), `cli/release.rs` (abandon) | 1 |
| W-D `e2e-harness` | D6 (base: scaffolder + shims + current-behavior tests) | new `tests/e2e/` only | 1 |
| W-F `verify-phase` | D3 | `coordinator.rs`, `adapters/*` verify impls, journal events v5, `cli/release.rs` verify cmd | 2 (alone on the seam) |
| W-G `cli-canon` | exit codes §2 (+ `--help --json` if time) | CLI error mapping | 2 |

Wave 1 units are file-disjoint except `cli/release.rs` (W-A: cut/plan/resume fns; W-C: abandon
fn — different regions, union-resolve). W-F waits for all wave-1 merges (it touches journal
events and coordinator). W-D extends its scenario set in wave 2 to cover D3/D4 behavior.

Non-goals this round: `release-ci-publish-mode` (lane tail, feature), plan-store GC, any
speculative hardening excluded by the 2026-08-17 issue standard.
