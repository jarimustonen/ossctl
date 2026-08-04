# ossctl

Release & readiness coordinator: the deterministic engine that takes any repo to
OSS release quality — the AI-first Rust CLI behind the `/oss-*` Claude Code skill
family. `ossctl` owns the normalizer/validator for the project release contract
(`OSS-RELEASE.md`), repo-fact detection, the readiness audit, and the resumable
per-ecosystem release-cut state machine; the prose `/oss-*` skills are thin callers
of this binary (the binary is the source of truth, §17).

## CLI Design Principles

This project follows the AI-first CLI conventions in [`AGENTS-AI-FIRST-CLI.md`](AGENTS-AI-FIRST-CLI.md) — strict input validation, `--json` output, JSONL logs, no interactive prompts, informative errors, composable commands. Read that file before designing or changing CLI surface. The file is a verbatim copy from `homebase`; treat it as shared canon, not a project-local doc to edit.

## Architecture (decided before code — read first)

The founding architecture is **already settled** in three accepted ADRs under
[`docs/adr/`](docs/adr/) — read them before writing any code; they are the spec, not
background:

- [`0001-founding-architecture.md`](docs/adr/0001-founding-architecture.md) — CLI
  command taxonomy (`contract` / `facts` / `audit` / `release …` / `skill` / `doctor`
  / `version`), the two-crate cargo workspace (`ossctl-core` lib + `ossctl-cli` bin),
  and the binary↔skill boundary for all 10 family members.
- [`0002-release-engine-adapter-model.md`](docs/adr/0002-release-engine-adapter-model.md)
  — the `ReleaseAdapter` trait + enum-backed registry, the phase-barrier coordinator
  (dry-run-all → build-all → publish-all → tag-once, coordinator-only tagging), and the
  sealed content-addressed `plan_id` approval seam.
- [`0003-config-and-journal-storage.md`](docs/adr/0003-config-and-journal-storage.md) —
  `OSS-RELEASE.md` stays the project contract; the event-sourced JSONL release journal
  under `git-common-dir/ossctl/releases/<run_id>/`; the remote-is-ground-truth
  resume/reconcile state table.

**Provenance.** `ossctl` extracts the deterministic core of a skill family designed in
`homebase` (`issues/oss-release-skill-family/`). The locked family design (`design.md`
there) is realized — not re-opened — by these ADRs. The already-built `/oss-init` unit
(a `SKILL.md`, `SCHEMA.md`, and two Python scripts `check-oss-release.py` /
`infer-repo-facts.py`) migrates into this repo; the scripts become `ossctl contract
validate` and `ossctl facts`.

**Status: Private, early.** The architecture is decided; the workspace is not yet
scaffolded. Open an `issuectl` issue before building a feature — do not pre-design the
app beyond what the ADRs already fix.

## Operating policy (for `/stint`)

`/stint` reads this section for how to run a work-session in this repo.

- **Green gate** (must pass before a unit counts as landed):
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo build --workspace` (release build not required per-unit)
- **Deploy: none per stint.** `ossctl` is a CLI library/binary; units land on `main`,
  they are not deployed to a server. *Publishing `ossctl` itself* (crates.io / GitHub
  Release / Homebrew) is a deliberate release act, never an automatic per-stint step —
  so `/stint` **skips Phase 4 (deploy)** here and says so.
  - **0.1.0 shipped 2026-08-04** (dogfood self-cut): crates.io `ossctl` + `ossctl-core`,
    GitHub Release `v0.1.0`, Homebrew tap `jarimustonen/homebrew-ossctl` (source-build
    formula). Repo is now **public**.
  - **Releases are currently HAND-DRIVEN**, not via `ossctl release cut` — the self-cut
    proved the engine can't yet publish a multi-crate workspace (dep order + index wait),
    bootstrap a first Homebrew formula, or build cross-platform (issues
    `cargo-adapter-workspace-publish`, `homebrew-adapter-first-formula`,
    `gh-release-ci-workflow`). Until those land, the recipe is: `cargo publish -p
    ossctl-core` → wait for index → `cargo publish -p ossctl`; `git tag vX.Y.Z && git push
    --tags`; `gh release create`; bump the tap formula's `url`+`sha256`. Landing those three
    issues makes v0.1.1 the first ENGINE-driven cut.
- **Cross-platform is a hard requirement (macOS AND Linux).** All software the `/oss-*`
  family produces — and `ossctl` itself — MUST install and run on **both macOS and Linux**
  (arm64 and x86_64). This is `/oss-*` family canon, not a nice-to-have: a release path
  that works on only one OS is incomplete. In practice that means every shipped tool
  offers a source path (`cargo install` / equivalent) plus prebuilt binaries and installers
  covering macOS (arm64 + x86_64) and Linux (statically-linked `musl`, arm64 + x86_64). For
  `ossctl` this is wired via `dist-workspace.toml` (cargo-dist) and the Homebrew tap
  (macOS + Linuxbrew). Treat a macOS-only or Linux-only install story as a release gap.
- **No Code of Conduct — deliberate.** This project intentionally ships **no**
  `CODE_OF_CONDUCT.md` (maintainer decision: no value seen). `ossctl audit` lists it as a
  `recommended` gap — that is **expected and accepted**; do **not** propose adding one or
  treat its absence as a defect.
- **Live-version check:** `ossctl version --json` (once the binary builds); before that,
  `git log --oneline` against `main`.
- **Hot files.** Two classes — do not treat them the same (learned across parallel
  rounds #3 and #5):
  - **Append-union-safe — parallel is fine.** The workspace/crate `Cargo.toml`, a module
    `mod.rs`, a CLI subcommand-dispatch file, and the bundled-skill `CATALOG` in
    `crates/ossctl-cli/src/skill.rs` collide only as *append* conflicts (a new dep line,
    a new `pub mod`, a new match arm, a new `BundledSkill { … }` row). Disjoint units may
    run in parallel against these — just brief each worker to **union-resolve** the conflict
    (keep all deps / all module decls / both arms / all rows). This resolved automatically in
    practice for the release campaign (`f-coordinator`↔`f-verify-cmd`). Do **not** serialize
    units solely because they both touch `Cargo.toml`.
    - **But the auto-merge is NOT guaranteed** (learned stint #6, prose-skills). A parallel
      worker's own auto-merge can *stall* on the union conflict: `f-changelog` authored complete
      green work but its `run merge` never completed — the run sat at `pending` because its
      branch (forked from `main`) hit a `skill.rs` CATALOG conflict after five siblings had
      advanced the integration branch. The worker did **not** union-resolve it despite the brief.
      So: parallelise freely, but **expect to salvage the last-in-line row-adder** — union-merge
      its clean commit by hand (keep all rows, re-run the green gate incl. the §17 lockstep gate,
      commit) — or serialize just the CATALOG-touching merges. The parallel *authoring* is safe;
      only the final *merge* of the append-file is not automatic.
  - **True shared-logic — sequence strictly, never parallelise.** A change to one of
    these is semantic, not an append, and a parallel edit means a real conflict:
    - `crates/ossctl-core/src/contract/schema.rs` — the ONE canonical serde model
    - a shared `crates/ossctl-core/src/protocol/*.rs` module two units both edit
      (a NEW `protocol/<x>.rs` per unit is append-safe; editing an existing shared one is not)
    - `crates/ossctl-core/src/release/coordinator.rs` and
      `crates/ossctl-core/src/release/adapters/mod.rs` — the release-engine seam
      (`EffectCtx` / `ReleaseArtifacts`, the phase-barrier coordinator). Semantic, not an
      append (learned stint #8: LANE R's two units both edited the artifact-threading seam,
      so they were sequenced strictly — parallelising them would have been a real conflict).
    - the canonical-JSON contract shape (SCHEMA) — the inter-skill contract; a change
      here ripples to every member
- **Migration rule:** the canonical-JSON output shape is a schema-versioned compatibility
  contract (§10). Preserve it; bump `schema_version` on a breaking change, never silently.
- **Test-account reset:** n/a (no external test accounts).

## Gitignored directories

- `history/` — agent scratchpad and ephemeral planning docs (not tracked)

## Documentation Pattern

Every directory follows this structure:

- `CLAUDE.md` — symlink to `AGENTS.md`
- `AGENTS.md` — all AI-relevant info (consolidated)
- `AGENTS-<TOPIC>.md` — complex topics split out (optional)

## Issues & Planning

Issue tracking is managed by [`issuectl`](https://github.com/jarimustonen/issuectl). Use the `/issue` skill (installed by `issuectl init`) to create, search, update, and close issues.

- `issues/<slug>/item.md` — every issue and epic (flat layout — no numeric prefix, no `open/closed/` split)
- Status lives in the `status:` frontmatter field, not in the path
- `issues/AGENTS.md` — issue schema, types, workflow (owned by issuectl)
- `.issuectl/AGENTS.md` — repo-local policy for AI agents (owned by issuectl)

All planning documents (plans, analyses, validations, designs, breakdowns, todos) belong under their parent issue directory — not as standalone files. If work needs a planning document, it also needs an issue.

- `issues/<slug>/plan.md` — architecture, implementation plans
- `issues/<slug>/analysis.md` — research and analysis
- `issues/<slug>/validation.md` — design assumptions checked against current reality, noting what differs from first-pass analysis
- `issues/<slug>/design.md` — design documents
- `issues/<slug>/breakdown.md` — epic → child-issue breakdown with dependencies and critical path
- `issues/<slug>/todo.md` — task checklists
