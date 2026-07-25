# ossctl founding ADRs (staging)

These are the **founding architecture decisions for the `ossctl` repo** (`~/Sources/ossctl`),
authored here as staging **before** the repo is scaffolded. `/create-project` carries them into
`ossctl/docs/adr/` as the repo's founding ADRs. They are written portably (they refer to the tool
as `ossctl` and assume the ossctl repo context, not homebase).

They realize — not re-open — the locked family design (`../design.md`) and conform to
`AGENTS-AI-FIRST-CLI.md`. They were pressure-tested with a four-lens `/llm-panel`
(architect / maintainability / AI-first-CLI-ergonomics / release-engineering).

| ADR | Decides |
|---|---|
| [0001](0001-founding-architecture.md) | **Founding spine** — CLI command taxonomy (noun-verb; `contract`/`facts`/`audit`/`release`/`skill`/`doctor`/`version`), the two-crate `ossctl-core` + `ossctl-cli` workspace, and the binary↔skill boundary for all 10 `/oss-*` members (all relocate into `ossctl`). |
| [0002](0002-release-engine-adapter-model.md) | **Release engine** — the `ReleaseAdapter` trait + enum-backed registry, the phase-barrier coordinator (dry-run-all → build-all → publish-all → tag-once, coordinator-owned tagging), and the sealed content-addressed `plan_id` approval seam that lets a non-interactive binary gate a partially-irreversible publish. |
| [0003](0003-config-and-journal-storage.md) | **Config + state storage** — `OSS-RELEASE.md` stays the project contract (not `ossctl`'s §8 tool config); the release-cut journal is event-sourced JSONL (`manifest.json` + events + `applied_seq` + idempotent reducer, mirroring `octl-core`) under `git rev-parse --git-common-dir`/ossctl/releases/`<run_id>`, with a documented remote-reconciliation state table. |

Cross-referenced to the issue via `Refs-Issue: oss-release-skill-family`.
