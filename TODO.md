# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-17 (stint #22 wrap). New agent: read this, then continue with a
fresh `/stint-start`. Main is clean + pushed. Live: **0.7.0 on all four channels,
verify-phase-observed AND independently spot-checked** (crates.io ×2, GitHub Release 11
assets, tap formula 0.7.0 + marker, local `brew` install runs)._

_**Where things stand.** The release-safety THEME ("the engine reports success without
verifying the artifact") is CLOSED: 0.7.0 shipped the durable plan store, the mandatory
verify phase, the `undeclared_distribution` refusal, stale-lock recovery in `abandon`, the
hermetic e2e harness (`crates/ossctl-cli/tests/e2e/`), and canon exit codes. The 0.7.0 cut
itself was the acceptance: the first `--bump` engine cut, invoked WITHOUT the `--bump`
flag, seven phases green ending in verify (run `01M07BDD2RBRADH70DYR41JPBA`). Design:
`issues/release-tag-preempts-cargo-dist/design.md` (D1–D6). Cross-repo audit + fleet
normalization: `homebase/issues/cross-repo-release-standardisation/audit-2026-08-17.md`._

_**THE 1.0 PLAN (maintainer-directed re-plan, 2026-08-17 wrap).** Lanes were restructured
by HOT-FILE FAMILY (not theme) so the waves parallelise correctly. **GLOBAL HEAD-OF-LINE:
`homebrew-ci-delegated-adapter`.**_

_**Wave A (next stint, 4 parallel workers → cut 0.8.0):**_
_- `contract-engine` lane head: **`homebrew-ci-delegated-adapter`** (HIGH) — spans BOTH
  strict seams (schema.rs + adapters), so it owns them alone this wave. Unlocks declaring
  the four fleet repos' CI-owned taps._
_- `bump-exec` lane: **`bump-single-crate-manifest`** (HIGH; bump_exec.rs only) — glasspad
  is a single-crate repo, so this blocks its future engine `--bump` path._
_- `plan-seal` lane: **`plan-phases-omit-verify`** (plan.rs only; mind the SEAL_VERSION
  rule if the phase list is in the seal pre-image)._
_- `cli-canon` lane head: **`intake-bug-ossctl-d9b2ec7bb6d9`** (`--version` alias)._
_- Wave close: engine-cut **0.8.0**, then declare the homebrew target in
  issuectl/glasspad/orchestratectl/project-canon contracts (cross-repo act, orchestrator)._

_**Wave B (following stint, sequenced in `contract-engine` → cut 0.9.0):**_
_- **`release-ci-publish-mode`** (HIGH) — glasspad's and orchestratectl's path onto the
  engine (glasspad's AGENTS forbids local publish; see the audit). Alone on the
  coordinator seam. NOTE: prefer the STRONGER worker model here (AGENTS worker-model note)._
_- **`publish-none-unrepresentable`** — schema/normalizer; unblocks intakectl's contract
  approval. Sequenced with the above (both touch schema.rs)._
_- **`cli-canon-help-json`** in parallel (cli surface only)._
_- Wave close: engine-cut **0.9.0**; move glasspad + orchestratectl release doctrines onto
  the engine; flip intakectl's contract to approved._

_**1.0 GATE (after Wave B):** issue base empty; one clean engine cut per fleet shape
(multicrate ✓ ossctl, single-crate `--bump`, CI-publish, publish-none, CI-delegated tap
verify); a ~2-week soak of routine fleet releases with zero new HIGH findings; then write
the 1.0 stability contract (which JSON shapes / exit codes / store formats are frozen) and
cut 1.0. Estimate at current pace: 2–4 weeks._

_**Open questions for the maintainer:** none pending. The Dependabot `clap` PR remains
open, untriaged. Known issuectl quirk (filed `intake-bug-issuectl-fab0edad2e42`): within a
lane, priority silently outranks `lane_seq` in the dag ordering._

**Read first (the spec):** `docs/adr/000{1,2,3,4}-*.md` + the AGENTS.md operating policy
(engine recipe, hot files, issue standard).

## Scheduling

Canonical scheduling lives in `issuectl` frontmatter (`lane:`, `lane_seq:`, `blocked_by:`,
`collision:`). Do not maintain a markdown DAG or adjacent backlog in this file.

Use these views instead:

```bash
issuectl dag
issuectl dag --json
issuectl ls --status open
issuectl ls --status in-progress
```

`TODO.md` is only the session handoff and project notes; issue bodies and `issuectl dag`
are the source of truth.

## Backlog

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN — closes when the
release-safety and cli-canon lanes drain). `issuectl list` for the live view.

## Piialiisan bugiraportit

- Intake reviewed through 2026-08-17. Latest: `intake-bug-ossctl-d9b2ec7bb6d9`
  (`--version` alias) admitted at the #22 wrap, now head of lane `cli-canon`. Older
  dispositions live in the issues themselves.
