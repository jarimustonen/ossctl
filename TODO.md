# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-01 (updated after stint #8). New agent: read this, then continue
with a fresh `/stint`. **NEXT STINT STARTS WITH `/oss-init` ON THIS REPO** (dogfood — see
NEXT ROUND). The Phase-4 build is complete AND the two SKELETON release adapters are now
finished — ossctl's own release path (crates.io + GitHub Release + Homebrew) is code-complete._

_STINT #8 LANDED ON MAIN — the `adapter-publish-completeness` campaign, decomposed into LANE R
and run as two sequenced spinoffs, both reviewed (4-model /llm-review + /assess-findings) and
green (271 tests):_
_- **`adapter-artifact-threading`** (`2f1ff8d` → `06b570c`, closed `fixed`) — coordinator now
  threads concrete release artifacts (asset paths + source tarball url/sha256) into every
  adapter's `publish()` via `EffectCtx::artifacts` / `ReleaseArtifacts` (adapters/mod.rs)._
_- **`adapter-skeletons-finish`** (`e25c8e1` → `a9138f3`, closed `fixed`) — `binary` (real
  GitHub-Release asset upload + real receipt, repo-pinned via `--repo`) and `homebrew` (real
  `bump-formula-pr --url`, `--sha256` passed only when honestly available). SKELETON markers
  GONE from binary.rs + homebrew.rs. Review correctly DROPPED a wrong pre-tag homebrew sha256._

_Earlier landed (stints #1–7): `workspace-scaffold`, `contract-command`, `facts-command`,
`skill-subcommand`, `audit-command`, ossctl-side of `migrate-oss-init`, the `release-engine`
epic (`613076d`), the adapter AUDIT (`d8518b5`), and `prose-skills` (`11b052c`) — the whole
/oss-* family (9 skills) is on `main`._

_REMAINING SKELETON markers in cargo/python/go/node adapters are BUILD-side (dist enumeration,
CI trusted-publisher jobs), NOT the publish path — they stay under the `adapter-publish-
completeness` umbrella (still in-progress) and are NOT needed for ossctl's own Rust release._

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

**NEXT ROUND (start here — decided with the user at end of stint #8): DOGFOOD ossctl on
itself.** The adapters are finished specifically so this can happen. The plan:

1. **`/oss-init` on this repo** — ossctl has NO `OSS-RELEASE.md` of its own yet. Run the
   generator (it reads Cargo manifests, git history, CI) to author a human-reviewable DRAFT
   config, then review/approve it. Targets: crates.io (`cargo`), GitHub Release binaries
   (`binary`), Homebrew (`homebrew`) — exactly the three adapters now code-complete.
2. Then `ossctl audit` → close readiness gaps → `ossctl release plan` → **`cut`** the real
   0.1.0 against live registries. This is the first REAL self-cut; expect to surface real-
   registry surprises the hermetic tests couldn't (that was always the point of finishing
   the adapters right before the cut).
3. Shipping 0.1.0 as an installable binary **unblocks `migrate-oss-init`** (homebase removal
   was deferred until `ossctl` is on PATH).

Secondary / non-urgent tails after the dogfood:
- **`adapter-publish-completeness`** (umbrella, in-progress) — finish the cargo/python/go/node
  BUILD-side skeletons (dist enumeration, CI publisher jobs). Not needed for ossctl's own Rust
  release; do when a non-Rust project needs those ecosystems.
- **`migrate-oss-init`** — remove the homebase `/oss-init` copy once ossctl is installed.

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

## Execution DAG (2026-08-02)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/7 (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

Stint #9 ran the dogfood `/oss-init`: `OSS-RELEASE.md` (draft) landed on main (`8bfdb87`),
validated clean by BOTH `check-oss-release.py` and ossctl's own `contract validate` (ports
agree). The dogfood surfaced the crates.io blockers, now the head-of-line coding unit:

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: prep-crates-io-publish
LANE C — workspace/crate Cargo.toml + OSS-RELEASE.md (append-union-safe hot files)
  ▶ prep-crates-io-publish   (rename CLI crate → ossctl, publish=true on both, add LICENSE, sync config)
LANE F — crates/ossctl-core/src/facts/mod.rs (disjoint from LANE C; parallel-safe)
  ▶ facts-workspace-members   (enumerate Cargo workspace members instead of package:null)
UNLANED — confirmed no shared hot files, run anytime (one slug per line):
    adapter-publish-completeness   (umbrella: cargo/python/go/node BUILD-side skeletons — non-urgent)
    migrate-oss-init               (blocked: needs ossctl installed on PATH)
```
<!-- execution-dag:end -->

Note: the head-of-line is the dogfood sequence (config authoring + real release cut), tracked
in NEXT ROUND above rather than as a DAG lane node — it produces `OSS-RELEASE.md` and cuts
0.1.0, it is not a hot-file coding unit. `adapter-publish-completeness` stays open only for
the non-Rust BUILD-side adapter skeletons. `migrate-oss-init` cannot start until ossctl
installs on PATH (which the dogfood cut delivers).

## Backlog

All Phase-4 units are children of the [`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md)
epic and blocker-wired (`issuectl show <slug>` shows `blocked_by`). `workspace-scaffold`
is the critical-path root. `issuectl list` for the live view. The core build is done; the
epic stays open only for the two remaining tails (see the Execution DAG above).
