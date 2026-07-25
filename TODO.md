# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-07-25. New agent: read this, then continue with a fresh `/stint`.
Repo just bootstrapped by `/create-project`; the founding architecture is decided and
the build backlog is filed. Start building._

**Focus:** Build `ossctl` — extract the `/oss-*` skill family's deterministic core into
this CLI, per the three founding ADRs in `docs/adr/`. The architecture is LOCKED; this is
implementation, dependency-first.
**Epic:** [`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md)
**Branch / worktree:** main (clean).

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
1. **[`workspace-scaffold`](issues/workspace-scaffold/item.md)** — the ONLY unblocked unit; do this first. Two-crate workspace + clap skeleton + `version`/`doctor` + CI.
2. [`contract-command`](issues/contract-command/item.md) — port `check-oss-release.py` → `ossctl contract show|validate` (the inter-skill contract; preserve JSON shape). *blocked by 1.*
3. [`facts-command`](issues/facts-command/item.md) — port `infer-repo-facts.py` → `ossctl facts`. *blocked by 1.*
4. [`skill-subcommand`](issues/skill-subcommand/item.md) — `ossctl skill list|install|print` + bundle mechanism (§15-17). *blocked by 1.*
5. [`audit-command`](issues/audit-command/item.md) — `ossctl audit` readiness engine. *blocked by 2+3.*
6. [`release-engine`](issues/release-engine/item.md) — plan/cut/resume/verify + adapters + journal (epic; ADR-0002/0003). *blocked by 2.*
7. [`migrate-oss-init`](issues/migrate-oss-init/item.md) — relocate `/oss-init`, delete its Python. *blocked by 2+3+4.*
8. [`prose-skills`](issues/prose-skills/item.md) — the skill-side members + `/oss-release` orchestrator. *blocked by 5.*

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
