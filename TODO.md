# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-07-25 (updated after stint #4). New agent: read this, then continue
with a fresh `/stint`. SIX units have LANDED: `workspace-scaffold`, `contract-command`
(canonical `contract/schema.rs` + `protocol` DTOs + `ossctl contract show|validate`),
`facts-command` (`ossctl facts`, byte-for-value identical to the Python detector),
`skill-subcommand` (`ossctl skill list|install|print` + §15-17 bundle mechanism + §17 CI
lockstep gate), `audit-command` (`ossctl audit` readiness engine — reads contract+facts,
read-only gap report, registry/GH failure⇒unknown), and the ossctl side of
`migrate-oss-init` (oss-init bundled as a skill template shelling out to the binary). 130
tests green on `main`. Now unblocked: `release-engine` (contract done) and `prose-skills`
(audit done). `migrate-oss-init` stays IN-PROGRESS — see the homebase-removal note below._

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

**NEXT ROUND (start here):** seven units effectively DONE. The `release-engine` epic was
delivered via `/orchestrate` (stint #5) as 7 features on the integration branch
**`orchestrate/release-engine-2026-07-26`** (HEAD 1abf4a2, 261 tests green, 0 deaths) — it is
NOT yet merged to main. **FIRST ACTION next session:** review that branch and merge it
(`/worktree-merge` from the worktree at `ossctl__worktrees/orchestrate-release-engine`, or a
direct `git merge`), then close the `release-engine` epic. Campaign report:
`~/.orchestratectl/runs/01kyfc8jf1x9rbf91kjfwdfssn/report.md`.
⚠️ Before a REAL release cut: audit each of the 6 ecosystem adapter `publish()` bodies — some
may be faithful skeletons (see report u-001 / spinoff s-001).

After that merge, the ONLY remaining backlog unit is
[`prose-skills`](issues/prose-skills/item.md): author the remaining prose /oss-* members + the
/oss-release orchestrator (all bundle under crates/ossctl-cli/skills/, ship via `ossctl skill
install`, none in homebase). It can start now (it depends on audit, which landed) but the
/oss-release-cut member wraps `release cut`, so it reads better AFTER the release-engine branch
merges. `migrate-oss-init` remains all-but-done (homebase removal deferred until ossctl is
installed on PATH — see above).

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
6. 🔀 [`release-engine`](issues/release-engine/item.md) — plan/cut/resume/verify + adapters + journal (epic; ADR-0002/0003). **DELIVERED via /orchestrate (stint #5) — 7 features on branch `orchestrate/release-engine-2026-07-26`, 261 tests green. AWAITS review+merge to main, then close epic.**
7. ⏳ [`migrate-oss-init`](issues/migrate-oss-init/item.md) — ossctl side DONE (stint #4, commits f8eb2fd → 97eaa0e); oss-init bundled as a skill template. **Homebase removal DEFERRED until ossctl is installed on PATH** (see handoff note) — issue kept in-progress.
8. [`prose-skills`](issues/prose-skills/item.md) — the skill-side members + `/oss-release` orchestrator. **UNBLOCKED — not yet started.**

**Watch out:**
- **No deploy step** — see AGENTS.md "Operating policy": units land on `main`, `/stint`
  skips Phase 4. Green gate = `cargo fmt --check` + `clippy -D warnings` + `test` + `build`.
- **Hot files** (sequence, don't parallelise): workspace/crate `Cargo.toml`,
  `contract/schema.rs`, `protocol/**`, the canonical-JSON contract. See AGENTS.md.
- **Preserve the canonical-JSON shape** from the oss-init `SCHEMA.md` §4 — it is a
  schema-versioned compatibility contract; bump `schema_version` on a break, never silently.
- The prose `/oss-*` skills currently referenced from homebase move here over time
  (`migrate-oss-init`, `prose-skills`); after migration, remove the homebase copies.

## Backlog

All Phase-4 units are children of the [`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md)
epic and blocker-wired (`issuectl show <slug>` shows `blocked_by`). `workspace-scaffold`
is the critical-path root. `issuectl list` for the live view.
