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

_**GLOBAL HEAD-OF-LINE: `homebrew-ci-delegated-adapter`** — it unlocks fleet-wide contract
uniformity (four repos' CI-owned taps become declarable + verify-observable) with less work
than `release-ci-publish-mode` (second in lane; it is glasspad's and orchestratectl's path
onto the engine). Parallel lane head: `intake-bug-ossctl-d9b2ec7bb6d9` (`--version` must
alias the `version` verb; maintainer-acked 2026-08-17; project-canon filed the same class
as its own `version-flag-alias`)._

_**Open questions for the maintainer:** none pending. The Dependabot `clap` PR remains
open, untriaged._

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
