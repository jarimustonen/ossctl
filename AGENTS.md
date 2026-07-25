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
