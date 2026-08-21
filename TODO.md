# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-21 (stint #24 wrap). New agent: read this, then continue with a
fresh `/stint-start`. Main is clean + pushed. Live: **0.10.0 on all four channels**, each
observed by the cut's own verify phase (crates.io ×2, GitHub Release 11 assets, tap
formula), CI green on the tag **and on main**, and the installed binary confirmed at
0.10.0._

_**Read the CI note below before you trust a local green gate.** This stint shipped 0.10.0
from a tree whose CI was red, because the documented gate passed locally and nobody checked
CI. The gate has since been made predictive (pinned toolchain), but the habit it exposed is
worth keeping: **check CI on main before cutting**, not just the local gate._

_**What this stint did.** Six units landed and one engine cut → 0.10.0. Wave A ran four in
parallel: the false-`missing` GitHub Release verification, the sealed-plan disposal gap in
`release abandon`, the Cargo publish evidence now exposed through `facts --json`, and the
plan/cut disagreement on repeated exact workspace pins. Wave B fixed the cargo-dist Homebrew
double-writer (plus a bounded tap-clone retry and observation-only verify after a post-tag
dist failure). A seventh unit then fixed red CI. Tests 746 → 768; every unit passed the full
green gate plus `/llm-review` before landing._

_**The strongest evidence this stint produced.** The 0.10.0 cut verified
`rust:ossctl:gh-releases (matches)` — the exact path that produced false reds on
project-canon v0.4.0 and v0.5.0. That fix is now confirmed against real infrastructure, not
just tests. Treat that as the model: this engine's bugs are only truly closed when a real
cut observes them._

_**And it was then confirmed against the original victim.** At wrap I re-ran the read-only
reconcile that `verify-gh-release-missing` cites as its decisive test —
`ossctl release verify 01M08P4D4HK25MRQXDE0XDW9NJ` in project-canon, the run that reported
`gh-releases: missing` on demand under 0.9.0. Under 0.10.0 it returns
`matches: 3, missing: 0, unknown: 0`. That closes the loop on the suspected cause too: the
issue predicted a package-vs-project naming mismatch (`project-canon-cli` vs `project-canon`),
and the fix — resolve the published tag, ignore cargo-dist's Release title, observe
cargo-dist's own manifest rather than inventing archive names — is exactly that class.
**Re-running an old journal's verify is a cheap, powerful regression check for this engine;
use it.**_

_**`SEAL_VERSION` is now 7** (was 6). The repeated-pin fix changed the meaning of a sealed
pin rewrite, so plans sealed by 0.9.0 or earlier can no longer start a fresh cut and must be
re-planned; legacy plans still load, so an interrupted run can still `resume`. This is the
second seal event in three releases (5→6 in 0.8.0) — expected, but check any fleet repo's
plan store for now-dead sealed plans._

_**Two process failures found this stint. Both matter more than any single bug fixed.**_

_1. **CI was red on main and 0.10.0 was cut anyway.** Two failures — an environment-dependent
   verify test (passed locally, failed on both CI runners) and a Clippy lint from a newer
   toolchain than the local one. The documented green gate therefore did not predict CI. Fixed
   by `ci-fails-release`: `rust-toolchain.toml` now pins 1.98.0 to match CI, the verify test
   was made hermetic by controlling the observer instead of depending on ambient network, and
   the AGENTS green-gate section now states both rules. The classification question underneath
   it was resolved deliberately and is worth knowing: **a destination that answers but lacks
   the artifact is `Missing`; a destination that cannot be reached or understood is `Unknown`**
   — absence of evidence is never evidence, per ADR-0002._

_2. **The CHANGELOG was corrupted before the cut and had to be repaired by hand.** Two of this
   round's entries had been merged into the already-published `[0.9.0]` section instead of
   `[Unreleased]`. Cutting from that state would have published wrong release notes for 0.10.0
   AND silently rewritten the history of a version already on crates.io, with the
   `SEAL_VERSION` break buried inside it. AGENTS.md lists `CHANGELOG.md` as append-union-safe,
   but a union merge does not catch a correct-looking entry in the **wrong section**. Worth
   considering a marker-anchored guard on released sections — `/oss-changelog` already uses
   markers for `[Unreleased]`. **Inspect the `[Unreleased]` block before every cut.**_

_**Infrastructure defect found outside ossctl (filed in homebase as `assess-models-wedges`).**
Two workers idled 11h28m and 5h38m inside `/assess-models` on an unbounded
`find /Users/jari`, and orchestratectl saw nothing wrong — healthy `pending` runs, no
`agent-died`, two `run wait` calls timing out normally. Killing the `find`s let both merge
within 143 seconds. The maintainer's decision is to move the model-performance corpus to
`haapa`; the filed issue records that the move alone does not close it, because the wedge
came from an improvised locator search, not from where the data lives. **If a worker in any
repo appears hung, capture its tmux pane and look for a long-running `find` before assuming a
model or harness problem.**_

_**Product direction — the 1.0 evidence gap is essentially unchanged.** The gate is one clean
engine cut per fleet shape. Multicrate-with-local-publish is now proven three times (0.8.0,
0.9.0, 0.10.0). The other three shapes — single-crate `--bump`, CI-publish, publish-none —
remain code-complete, well-tested, and **never run in a real release**. Do not treat tests as
substitute evidence; this stint is itself the argument, since the local gate was green while
CI was red. The cross-repo follow-through is still the most direct route to those missing
runs. After that: the ~2-week soak with zero new HIGH findings, then the 1.0 stability
contract (which JSON shapes / exit codes / store formats freeze) and the 1.0 cut._

_**The engine models three publishing dispositions in one vocabulary** — Engine
publishes / CI publishes and the engine observes / nothing is published at all. That is
the frame for reading the remaining work._

_**Worth knowing about the review that caught the most** (stint #23). The `release-ci-publish-mode`
unit ran on the stronger worker model per the AGENTS worker-model note, and its
`/llm-review` found three failure modes that would only have surfaced AFTER the
irreversible tag. One was silent: cutting an already-published version would have gone
green over a CI publish that failed with "already uploaded". All three are now refused
before the tag. Treat "what fails after the point of no return" as a standing review lens
on this engine._

_**Fleet work done outside ossctl (originated in stint #23; still the live picture).**
Contract-alignment issues were filed in five repos now that the engine features they need
have shipped: `issuectl` (an ACTIVE Homebrew double-writer that already false-red'd its
0.15.0 cut), `glasspad` and `orchestratectl` (under-declared surface + `cargo-publish`
contradicting their CI-publish reality), `project-canon` (under-declared surface), and
`intakectl` (contract can finally go `approved`). Each issue carries the target shape and
points at ossctl's own contract as the worked example, with an explicit table of the one
adapter line that must NOT be copied. Separately, the issuectl intake lifecycle was
enabled across all eight active repos (schema enum + `intake migrate --apply` +
`doctor --fix`): `via:*` labels became a structured `provenance:` field and label-encoded
triage state became real statuses. That surfaced **12 previously invisible untriaged bug
reports in deutschpad** — they were unreachable by any status query while their state
lived in a label._

_**Unresolved decisions for the maintainer:**_
_1. **Does ossctl converge to the uniform fleet shape?** The maintainer asked for uniform
   releases across the fleet (intakectl excepted). I kept ossctl on its engine-owned tap
   and local publish, because it is the only live exercise of the `homebrew-tap` adapter —
   converging it would leave that path with no real release testing it. Not yet ratified._
_2. **RESOLVED and no longer ossctl's to carry — `project-canon` publishes from CI.** The
   answer was settled from that repo's own files, not guessed: `.github/workflows/
   publish-crates.yml` is tag-triggered (`on: push: tags: ['v[0-9]+...']`) and its header
   states "crates.io publishing happens in CI with no dependency on a local token". So its
   contract's `adapter: cargo-publish` on both crates contradicts reality and would
   double-publish; `cargo-publish-ci` is correct. **A project-canon agent now owns that work
   (maintainer, 2026-08-21) — do not act on it from here.** Recorded only because ossctl's
   handoff previously listed it as an open question._
_3. **Cross-repo follow-through is PART DONE (verified 2026-08-20 at wrap).** Of the five
   filed issues, `issuectl` (`homebrew-double-writer-contract` — the active double writer)
   and `orchestratectl` (`contract-declare-ci-publish-surface`) are already `fixed`;
   orchestratectl's contract now declares `cargo-publish-ci` ×2 + `cargo-dist` for both
   gh-releases and homebrew. Still open: `glasspad`, `project-canon`, `intakectl`. Note
   NONE of the fixed ones has yet been through a real cut, so the 1.0 evidence gap noted above is
   unchanged — declaring the shape is not the same as proving it._
_4. **RESOLVED — `ossctl-phase4-build` was closed as delivered** (maintainer decision,
   2026-08-21). Every stage of its build order shipped, and it tracked no children. It was
   deliberately NOT recycled into a 1.0 tracker, because its scope had already drifted and an
   epic whose scope silently changes can never be audited. **Open question: the 1.0 gate now
   has no tracking artifact.** If one is wanted, it needs its own epic with a checkable
   condition — all four fleet shapes proven in real cuts (currently 1/4), a ~2-week soak with
   no new HIGH findings, and a written stability contract. Not yet filed._
_5. Housekeeping left deliberately undone: `issuectl` lacks `.issuectl/AGENTS.md`
   (`issuectl agents init` is an opt-in, not a fix) and has 2 broken cross-references + 1
   unknown frontmatter key; `deutschpad` has 20 closed issues missing `closed:` dates that
   `doctor` cannot invent. Dependabot PRs remain open and untriaged — note the clap 4.6.6 PR
   surfaced the red-CI regressions before anyone else did, so they are worth a look._

_**What remains, and the shape of it.** Both of last stint's dominant HIGH false-reds are
fixed and released. The open work now splits in two:_

_**The verify seam** still has four items, and the through-line is that verify reasons about
**destinations** rather than about the **delegated run that fills them**. The sharpest is
`verify-delegated-run-state`, from a real issuectl 0.16.0 cut: "still building", "succeeded",
and "died" are indistinguishable, so a cancelled cargo-dist workflow (six-hour runner queue,
then GitHub's job ceiling) reported only `gh-releases (missing)` and the operator's plausible
first reading — "verify raced CI" — was wrong and cost a round-trip. It was placed ahead of
`delegated-verify-window-ux` on purpose: that item rebuilds the same polling loop, and the
loop is far easier to write once verify can separate pending from failed. `delegated-registry-
verify-destination` and `delegated-publish-workflow-preflight` complete the seam. Note the
CI-fix unit touched adjacent classification code — check whether it partly subsumes the
registry-destination item before implementing it fresh._

_**The bump/seal path** carries `bump-inherited-workspace-pins`, which is the same class as
the repeated-pin bug fixed this stint but **silent where that one was loud**: a member
inheriting an exact internal pin from root `[workspace.dependencies]` publishes to crates.io
tied to a stale internal version while local checks stay green, because they resolve through
`path`. Not reachable in ossctl's own contract (its `[workspace.dependencies]` holds only
external crates), but reachable by any downstream repo using that very common layout. Its fix
extends the same discovery path just reconciled, so check whether it moves the seal pre-image
again — that would be another deliberate `SEAL_VERSION` event, not a silent hash change._

_**Cross-repo issue filing does NOT work — plan for the fallback.** Reproduced again
2026-08-21 during this wrap. `intakectl file --repo <other>` still lands a `tg-bug-…` slug in
**homebase**, and the slug guard then refuses to confirm (the gate holds — nothing reaches any
remote). Diagnosis has advanced though, and it matters: the **client half is fixed** (it
computed the correct `intake-bug-orchestratectl-…` slug, on the deployed binary at the very
commit that closed the bug, `8d6bc96`), so the remaining defect is the **server-side
deterministic filer**. Reopened as intakectl `intake-file-routing` with that evidence; inbox
entry `0fed8f9c-df4b-47d4-863c-9fdb946e02cc` was parked by the failed attempt and needs
disposal. **Until it is fixed, file directly in the target tool's own repo** — that is what
this wrap did for `run-wait-json` / `worker-wedged-one` (orchestratectl) and
`intake-file-generates` (issuectl). A second filer defect
(`intake-filer-legacy-label-shape`) means freshly filed items still get `status: open` + a
`needs-triage` label instead of `status: untriaged`._

_**Known issuectl quirk** (filed `intake-bug-issuectl-fab0edad2e42`): within a lane,
priority silently outranks `lane_seq` in dag ordering — visible in `issuectl dag`'s own
"intra-lane order" line. Set priority deliberately, not just sequence._

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

[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) — the founding extraction epic —
was closed as delivered on 2026-08-21. There is no parent epic now; open work stands on its
own issues. `issuectl list` for the live view.

## Piialiisan bugiraportit

- [x] 🐛 Piialiisan bugiraportti: Release bump plan accepts duplicate exact pins then cut fails — FIXED and released in 0.10.0 (SEAL_VERSION 6->7) — jari via Telegram ([`intake-bug-ossctl-d38ddf598fd5`](issues/intake-bug-ossctl-d38ddf598fd5/item.md))
- [x] 🐛 Piialiisan bugiraportti: Cargo-dist verifier reports existing GitHub Releases missing — closed as a duplicate; evidence folded into `verify-gh-release-missing` — jari via Telegram ([`intake-bug-ossctl-09cd3c1d03d0`](issues/intake-bug-ossctl-09cd3c1d03d0/item.md))
