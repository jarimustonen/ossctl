# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-07-31 (updated after stint #7). New agent: read this, then continue
with a fresh `/stint`. The Phase-4 BUILD is now COMPLETE on `main`; only two non-urgent
tails remain (both in-progress, neither actionable/urgent this round — see below).
LANDED ON MAIN: `workspace-scaffold`, `contract-command`, `facts-command`, `skill-subcommand`,
`audit-command`, ossctl-side of `migrate-oss-init`, the **`release-engine` epic** (`613076d`),
the **adapter-publish-completeness AUDIT** (`d8518b5`, read-only), and — NEW this stint #7 —
**`prose-skills`**: merged to main (merge `11b052c`, clean text-merge of the campaign branch;
261 tests green, fmt+clippy clean, all 9 /oss-* skills bundle + print). `prose-skills` is
CLOSED. The whole /oss-* family (9 skills) is now on `main`._

_EPIC STATUS — deliberately NOT closed. The stint #6 handoff said to close
`ossctl-phase4-build` after the prose-skills merge; stint #7 checked ground truth and left it
OPEN, because it still has two genuinely in-progress children (`migrate-oss-init`,
`adapter-publish-completeness`). Close the epic only when BOTH tails resolve — do not close it
just because the core build compiles._

_migrate-oss-init tail (DO NOT rush): the ossctl side is done + bundled, but the homebase
copy (`dotfiles/src/.claude/skills/oss-init/`: SKILL.md, SCHEMA.md, scripts/*.py, fixtures/)
is STILL the user's live /oss-init. Removing it now would break /oss-init in the environment,
because ossctl is not yet installed on PATH — `ossctl skill install oss-init` can't take over
until ossctl ships as a distributable binary. So the homebase removal is DEFERRED until ossctl
is installed. Keep migrate-oss-init open until then. Homebase design docs in
issues/oss-release-skill-family/ STAY (history); only the live skill + Python get removed,
and only after ossctl install works._

_Stint-note (agent-death + salvage): the first `contract-command` spinoff DIED (`agent-died`)
at ~31 min AFTER committing complete green work, before its review+merge — the underlying
agent PROCESS exited (verified: orx watchdog `kill(pid,0)` correctly detected it; NOT a false
reap, NOT an orx defect). Salvaged via a follow-up spinoff (fast-forward the stranded commit →
/llm-review → merge), which caught a real floor-bypass bug (`../` escape). Filed the orx-side
gap as `agent-death-strands-recoverable-work` (orx repo, `aa9aff9`): a dead agent's
committed-but-unmerged clean work is stranded as a plain failure with no recoverability
signal. Jari is fixing that with the orx agent. STANDING PROTOCOL until it lands: on any
spinoff that settles `failed`, check `git log main..<branch>`; if clean+green, auto-salvage
(bring in commit → /llm-review → merge) rather than discarding. Parallel round #3
(facts+skill together) landed clean with NO deaths — so the ~31-min death is one data point,
not yet a pattern._

**Focus:** Build `ossctl` — extract the `/oss-*` skill family's deterministic core into
this CLI, per the three founding ADRs in `docs/adr/`. The architecture is LOCKED; this is
implementation, dependency-first.
**Epic:** [`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md)
**Branch / worktree:** main (clean).

**NEXT ROUND (start here):** the core build is done and both remaining items are NON-URGENT.
There is no forced next action. When you next work this repo, the two candidate tails are:

- **`adapter-publish-completeness`** (in-progress, open) — the AUDIT landed;
  `issues/adapter-publish-completeness/analysis.md` scopes the completion work: **3 REAL
  adapters** (cargo, python, go), **1 PARTIAL** (node), **2 SKELETON** (homebrew, binary — both
  blocked on one coordinator change that threads asset paths / tarball-URL+sha256). Recommended
  order in analysis.md: cross-cutting decisions (auth=ambient-env, idempotency=coordinator
  verify pre-check, **artifact-threading — the single biggest missing piece**) →
  cargo/python/go (parallel) → node → binary → homebrew. This is a small dependency-ordered
  campaign (the coordinator change gates the two skeletons), so it leans toward `/orchestrate`,
  NOT one Phase-3 spinoff. **Best done right before the first REAL self-cut** — you'll want to
  test against live registries then, which is when finishing the adapters actually pays off.
  Recommended first unit if you do start: just the cross-cutting coordinator/`AdapterTarget`
  threading (everything else depends on it), then fan the per-adapter finishing out later.
- **`migrate-oss-init`** — all-but-done; homebase removal DEFERRED until ossctl is installed on
  PATH (see the tail note above). Not actionable until ossctl ships as a distributable binary.

Stint #7 was offered these and the user chose to STOP (build complete, tails not urgent) —
so no worktree was spawned this round. Both tails are documented and waiting.

**Hot-file learning (round #3):** crate `Cargo.toml` is listed as a hot file, but two
disjoint units (`facts-command` + `skill-subcommand`) ran in PARALLEL and merged clean — the
`Cargo.toml` dependency-append "collision" auto-resolved via union with no manual step. So
`Cargo.toml`-only overlap does NOT force serialization; brief each parallel agent to
union-resolve Cargo.toml conflicts and let them run. Reserve strict sequencing for real
shared-logic files (`contract/schema.rs`, a shared `protocol/*.rs` module).

**Read first (the spec):**
- `docs/adr/0001-founding-architecture.md` — CLI taxonomy, two-crate workspace, binary↔skill boundary.
- `docs/adr/0002-release-engine-adapter-model.md` — the release engine.
- `docs/adr/0003-config-and-journal-storage.md` — config artifact + journal.
- The locked family design lives in **homebase** `issues/oss-release-skill-family/design.md`
  (the ADRs realize it — read it for the family's *what*, the ADRs for ossctl's *how*).
- The already-built `/oss-init` unit is in **homebase** `dotfiles/src/.claude/skills/oss-init/`
  (`SKILL.md`, `SCHEMA.md`, `scripts/check-oss-release.py`, `scripts/infer-repo-facts.py`) —
  these migrate here; `SCHEMA.md` §4 is the canonical-JSON contract to preserve.

**Build order (ADR-0001 dependency-first) — the backlog is filed and blocker-wired:**
1. ~~**[`workspace-scaffold`](issues/workspace-scaffold/item.md)** — Two-crate workspace + clap skeleton + `version`/`doctor` + CI.~~ ✅ **DONE** (stint #1, commits b939fa7 → 1bb18ab).
2. ~~[`contract-command`](issues/contract-command/item.md) — port `check-oss-release.py` → `ossctl contract show|validate` (the inter-skill contract; preserve JSON shape).~~ ✅ **DONE** (stint #2, commits ee39196 → 7f07930; salvaged after agent-death, reviewed).
3. ~~[`facts-command`](issues/facts-command/item.md) — port `infer-repo-facts.py` → `ossctl facts`.~~ ✅ **DONE** (stint #3, commits 7f9fe99 → 9450b2b).
4. ~~[`skill-subcommand`](issues/skill-subcommand/item.md) — `ossctl skill list|install|print` + bundle mechanism (§15-17).~~ ✅ **DONE** (stint #3, commits 4f7e7a6 → 0103247).
5. ~~[`audit-command`](issues/audit-command/item.md) — `ossctl audit` readiness engine.~~ ✅ **DONE** (stint #4, commits 0d54998 → b423cd0).
6. ~~[`release-engine`](issues/release-engine/item.md) — plan/cut/resume/verify + adapters + journal (epic; ADR-0002/0003).~~ ✅ **DONE** — delivered via /orchestrate (stint #5), **merged to main stint #6** (`613076d`, 261 tests green); epic CLOSED.
7. ⏳ [`migrate-oss-init`](issues/migrate-oss-init/item.md) — ossctl side DONE (stint #4, commits f8eb2fd → 97eaa0e); oss-init bundled as a skill template. **Homebase removal DEFERRED until ossctl is installed on PATH** (see handoff note) — issue kept in-progress.
8. ~~[`prose-skills`](issues/prose-skills/item.md) — the skill-side members + `/oss-release` orchestrator.~~ ✅ **DONE** — delivered via /orchestrate (stint #6), **merged to main stint #7** (merge `11b052c`, 261 tests green, all 9 skills bundle); issue CLOSED.
9. ⏳ [`adapter-publish-completeness`](issues/adapter-publish-completeness/item.md) — follow-up from release-engine. **AUDIT landed (stint #6, `d8518b5`)**; `analysis.md` scopes completion (3 REAL / 1 PARTIAL / 2 SKELETON). Do before a REAL release cut — not urgent.

**Watch out:**
- **No deploy step** — see AGENTS.md "Operating policy": units land on `main`, `/stint`
  skips Phase 4. Green gate = `cargo fmt --check` + `clippy -D warnings` + `test` + `build`.
- **Hot files** (sequence, don't parallelise): workspace/crate `Cargo.toml`,
  `contract/schema.rs`, `protocol/**`, the canonical-JSON contract. See AGENTS.md.
- **Preserve the canonical-JSON shape** from the oss-init `SCHEMA.md` §4 — it is a
  schema-versioned compatibility contract; bump `schema_version` on a break, never silently.
- The prose `/oss-*` skills currently referenced from homebase move here over time
  (`migrate-oss-init`, `prose-skills`); after migration, remove the homebase copies.

## Execution DAG (2026-07-31)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/7 (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

The Phase-4 build is complete; only two non-urgent tails remain, and they are disjoint
(no shared hot file), so both are UNLANED. `migrate-oss-init` is externally blocked (needs
ossctl on PATH) — not spawnable until then.

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: adapter-publish-completeness   ← if a round starts; else nothing is urgent
UNLANED — confirmed no shared hot files, run anytime (one slug per line):
    adapter-publish-completeness
    migrate-oss-init
```
<!-- execution-dag:end -->

Note: `adapter-publish-completeness`, if started, is itself a small dependency-ordered
campaign (coordinator threading gates the two skeleton adapters) — treat it as an
`/orchestrate` hand-off, not one spinoff. `migrate-oss-init` cannot start until ossctl
installs on PATH.

## Backlog

All Phase-4 units are children of the [`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md)
epic and blocker-wired (`issuectl show <slug>` shows `blocked_by`). `workspace-scaffold`
is the critical-path root. `issuectl list` for the live view. The core build is done; the
epic stays open only for the two remaining tails (see the Execution DAG above).
