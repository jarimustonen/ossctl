# ossctl founding ADRs (staging)

These are the **founding architecture decisions for the `ossctl` repository**,
authored as staging **before** the repository was scaffolded. They are written portably: they
refer to the tool as `ossctl` and assume the ossctl repository context.

They realize — not re-open — the locked family design (`../design.md`) and conform to
`AGENTS-AI-FIRST-CLI.md`. They were pressure-tested with a four-lens `/llm-panel`
(architect / maintainability / AI-first-CLI-ergonomics / release-engineering).

| ADR | Decides |
|---|---|
| [0001](0001-founding-architecture.md) | **Founding spine** — CLI command taxonomy (noun-verb; `contract`/`facts`/`audit`/`release`/`skill`/`doctor`/`version`), the two-crate `ossctl-core` + `ossctl-cli` workspace, and the binary↔skill boundary for all 10 `/oss-*` members (all relocate into `ossctl`). |
| [0002](0002-release-engine-adapter-model.md) | **Release engine** — the `ReleaseAdapter` trait + enum-backed registry, the phase-barrier coordinator (dry-run-all → build-all → publish-all → tag-once, coordinator-owned tagging), and the sealed content-addressed `plan_id` approval seam that lets a non-interactive binary gate a partially-irreversible publish. |
| [0003](0003-config-and-journal-storage.md) | **Config + state storage** — `OSS-RELEASE.md` stays the project contract (not `ossctl`'s §8 tool config); the release-cut journal is event-sourced JSONL (`manifest.json` + events + `applied_seq` + idempotent reducer, mirroring `octl-core`) under `git rev-parse --git-common-dir`/ossctl/releases/`<run_id>`, with a documented remote-reconciliation state table. |
| [0004](0004-cargo-adapter-one-target-one-publish-unit.md) | **Cargo adapter: one plan target = one publish unit** — the cargo adapter publishes only its own target's crate (waiting on that crate's workspace deps to index first); the coordinator owns all cross-target ordering. Removes the closure-per-target double-publish/partial-publish trap once a contract declares >1 crates.io target, makes `is_published` tri-state (fail-closed on registry outage), and fixes the target model to "each publishable crate is its own declared target". (Post-founding; surfaced by a `/llm-review` of `release-cut-multi-target-ecosystem`.) |
| [0005](0005-shipshape-product-migration.md) | **Shipshape migration** — canonical product/crate/skill identities, frozen legacy package policy, actionable old-skill refusals, permanent compatibility storage and seal namespaces, wire/journal compatibility, and the post-merge channel/machine rollout order. |

ADRs 0001–0004 intentionally retain the historical `ossctl` name. ADR-0005 supersedes
that product identity without rewriting earlier decisions.

The founding ADRs (0001–0003) are cross-referenced to the issue via `Refs-Issue: oss-release-skill-family`; 0004 via `Refs-Issue: cargo-adapter-multitarget-double-publish`.
