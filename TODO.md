# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-20 (stint #23 wrap). New agent: read this, then continue with a
fresh `/stint-start`. Main is clean + pushed. Live: **0.9.0 on all four channels**, each
observed by the cut's own verify phase (crates.io ×2, GitHub Release 11 assets, tap
formula), CI green on the tag, and `ossctl --version` confirmed against a fresh install._

_**What this stint did.** Two full waves and two engine cuts in one session. Wave A →
0.8.0: the CI-delegated Homebrew target, the single-crate `[package] version` bump
fallback, `verify` added to the sealed plan's phase list (a deliberate `SEAL_VERSION`
5→6 event — plans sealed before 0.8.0 need re-planning), and `--version` / `-V` as
aliases of the `version` verb. Wave B → 0.9.0: the `cargo-publish-ci` tag-only publish
mode, publish-none (`targets: []`) as a first-class cuttable shape, `--help --json` for
the whole command tree, and centralized stdout writing so a broken pipe exits quietly
instead of panicking. Test count went 686 → 746; every unit passed the full green gate
plus `/llm-review` before landing._

_**The engine now models three publishing dispositions in one vocabulary** — Engine
publishes / CI publishes and the engine observes / nothing is published at all. That is
the through-line of both cuts and the frame for reading the remaining work._

_**Worth knowing about the review that caught the most.** The `release-ci-publish-mode`
unit ran on the stronger worker model per the AGENTS worker-model note, and its
`/llm-review` found three failure modes that would only have surfaced AFTER the
irreversible tag. One was silent: cutting an already-published version would have gone
green over a CI publish that failed with "already uploaded". All three are now refused
before the tag. Treat "what fails after the point of no return" as a standing review lens
on this engine._

_**Fleet work done outside ossctl (recorded here because it originated in this stint).**
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

_**Product direction — the 1.0 gate is closer but one condition is genuinely unmet.** The
gate is one clean engine cut per fleet shape. Multicrate-with-local-publish is proven
twice (ossctl 0.8.0, 0.9.0). The other three shapes — single-crate `--bump`, CI-publish,
publish-none — are code-complete and well-tested but have **never run in a real
release**. Do not treat tests as substitute evidence here; the failures this engine exists
to catch appear against real infrastructure under real timing. The cross-repo
follow-through above would produce exactly those missing runs as a by-product, which is
why it is the most direct route to 1.0 rather than a side quest. After that: the ~2-week
soak with zero new HIGH findings, then write the 1.0 stability contract (which JSON
shapes / exit codes / store formats freeze) and cut 1.0._

_**Unresolved decisions for the maintainer:**_
_1. **Does ossctl converge to the uniform fleet shape?** The maintainer asked for uniform
   releases across the fleet (intakectl excepted). I kept ossctl on its engine-owned tap
   and local publish, because it is the only live exercise of the `homebrew-tap` adapter —
   converging it would leave that path with no real release testing it. Not yet ratified._
_2. **`project-canon`: is its crates.io publish local or CI-performed?** Unlike
   orchestratectl its contract does not say, and guessing wrong either double-publishes or
   pushes a tag and waits 20 minutes for a publish nobody performs. Its filed issue
   deliberately refuses to guess._
_3. **Cross-repo follow-through is unauthorised.** The five filed issues are ossctl-scope
   clean but acting on them edits other repos' release behaviour. Awaiting a go._
_4. **`ossctl-phase4-build` (the parent epic) is unscheduled** and needs scheduling
   triage — it was written to close when the release-safety and cli-canon lanes drain, and
   both have since drained and refilled with different work._
_5. Housekeeping left deliberately undone: `issuectl` lacks `.issuectl/AGENTS.md`
   (`issuectl agents init` is an opt-in, not a fix) and has 2 broken cross-references + 1
   unknown frontmatter key; `deutschpad` has 20 closed issues missing `closed:` dates that
   `doctor` cannot invent. Five Dependabot PRs remain open and untriaged._

_**Two HIGH bugs dominate the remaining work, both false-reds on delivered releases.**
`verify-gh-release-missing` — a published GitHub Release verifies as `missing`, observed
four times on project-canon, confirmed present in the released 0.9.0, and it also leaves
runs stuck `in_progress`. It is a lookup bug, not a timing race (the release existed for
~18 of the 20 polling minutes). A starting-point hint is recorded on the issue:
`adapters/binary.rs` keys correctly on the tag, but the reported error text is
`reconcile.rs`'s wording and that path routes gh-releases through a generic registry
query. The hint is explicitly marked unverified — confirm it, do not inherit it.
`cut-runs-own` is the engine half of issuectl's double-writer, plus a missing 503 retry.
Also newly admitted: a sealed `--bump` plan counted one intra-workspace pin where the cut
found two and refused — plan and cut disagreeing about one sealed tree, which is a seal-
boundary problem rather than a bump edge case._

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

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN — closes when the
release-safety and cli-canon lanes drain). `issuectl list` for the live view.

## Piialiisan bugiraportit

- [x] 🐛 Piialiisan bugiraportti: Release bump plan accepts duplicate exact pins then cut fails — admitted to the plan — jari via Telegram ([`intake-bug-ossctl-d38ddf598fd5`](issues/intake-bug-ossctl-d38ddf598fd5/item.md))
- [x] 🐛 Piialiisan bugiraportti: Cargo-dist verifier reports existing GitHub Releases missing — closed as a duplicate; evidence folded into `verify-gh-release-missing` — jari via Telegram ([`intake-bug-ossctl-09cd3c1d03d0`](issues/intake-bug-ossctl-09cd3c1d03d0/item.md))
