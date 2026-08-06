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
  - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — **CI runs this and it
    is easy to miss locally**: broken intra-doc links (`[`Foo`]` to a moved/renamed/private
    item, redundant explicit link targets) fail the `docs` job even when tests pass. Run it
    before landing any unit that touches doc comments (`//!` / `///`).
- **Releases MAY be cut automatically whenever there is something to release** (maintainer
  decision, 2026-08-05). Publishing `ossctl` itself (crates.io / GitHub Release / Homebrew)
  no longer requires an explicit per-release go: when `main` carries unreleased user-facing
  changes, `/stint` may bump the version, finalize the CHANGELOG, and run the release recipe
  as an owned Phase-3 act — no confirmation needed. Preconditions still hold: the green gate
  passes, and `cargo publish` runs `--dry-run` first. crates.io publishes are irreversible
  (yank-only), so never publish red, and report each step.
- **The ENGINE-DRIVEN cut (`ossctl release cut`) is fully autonomous — NO go/no-go checkpoint,
  ever** (maintainer decision, 2026-08-06). Running the release *through the engine* — the full
  multi-target flow (crates.io ×2 + cargo-dist binaries + the Homebrew tap) — requires **no
  permission and no pause before the irreversible publish**, not for the first-ever engine cut,
  not for the homebrew leg (the homebrew leg is the most important target — it must be cut, not
  dropped). Do **not** stop to ask "shall I cut?" — just run the recipe end to end and report
  as you go. The safety is structural, not a human gate: `ossctl release plan` seals a
  content-addressed plan (a side-effect-free preview the agent inspects), the coordinator runs
  `dry-run-all` before any publish, `ossctl-core`→`ossctl` ordering + index-wait guard the
  crates.io partial-publish case, and `ossctl release resume`/`abandon` recover an interrupted
  run. Still: green gate first, dry-run/plan first, never publish red, report each phase.
  - **Shipped: 0.1.0 (2026-08-04), 0.1.1 (2026-08-05), 0.1.2 (2026-08-05), 0.2.0 (2026-08-06).**
    All on crates.io (`ossctl` + `ossctl-core`), GitHub Releases (cross-platform: macOS aarch64,
    Linux musl x86_64+aarch64, Windows, `.sh`+`.ps1` installers), and the Homebrew tap
    `jarimustonen/homebrew-ossctl`. Repo is **public**. 0.1.2 added `ossctl dist generate`. 0.2.0
    made the engine drive a multi-target cut (multi-target/ecosystem in dep order, one-target-one-
    publish-unit/ADR-0004, CI-delegated skip, GH-Release-to-CI, post-tag homebrew, crates.io-pin)
    + `release list`/`abandon`.
  - **0.2.0 was cut MANUALLY, not by the engine** — the two attempted engine cuts failed SAFELY
    at the build phase: `cargo package -p ossctl` resolves the `=`-pinned `ossctl-core` dep against
    the crates.io index (even with `--no-verify`), so a dependent can't be packaged until its dep
    is actually published. The `build-all → publish-all` barrier is incompatible with cargo's
    multi-crate `=`-pinned model; the real fix is to INTERLEAVE build+publish per dep-ordered cargo
    target (publish core → wait index → package+publish cli). Tracked (REOPENED, HIGH):
    `release-cut-build-phase-dep-ordering` — the last blocker before the engine can dogfood its own
    (0.2.1) cut. Until then, cut ossctl via the fallback manual recipe below.
  - **The ENGINE recipe (`ossctl release cut`) is the INTENDED primary path** (built stint #12),
    but is **blocked on `release-cut-build-phase-dep-ordering`** for ossctl's own multi-crate cut
    (see above) — so 0.2.0 shipped via the manual fallback; the engine recipe becomes primary once
    that blocker lands. ossctl's own `OSS-RELEASE.md` declares the
    four targets (ossctl-core + ossctl on crates.io; ossctl on gh-releases/cargo-dist; ossctl
    on homebrew) plus a `distribution` block with `homebrew_tap: jarimustonen/homebrew-ossctl`.
    The recipe:
    1. Bump `workspace.package.version` + the internal `=X.Y.Z` dep in lockstep → finalize
       CHANGELOG → `cargo build` (refresh lock) → `cargo publish -p ossctl-core --dry-run` →
       commit → push `main`.
    2. `ossctl release plan --version X.Y.Z` (seal + inspect; side-effect-free) → then
       `ossctl release cut --plan <id> --version X.Y.Z`. The engine runs
       `dry-run-all → build-all → publish-all (crates.io, dep-ordered, index-waited) → tag →
       dist (homebrew: real sha256 pushed to the tap)`. It **skips** the CI-delegated
       cargo-dist target and **delegates the GitHub Release to CI** (Option 1,
       `ci_owns_github_release`) — so the engine does crates.io + tag + homebrew; cargo-dist
       CI does the binaries + the GitHub Release. No manual `gh workflow run publish-crates.yml`
       and no manual homebrew bump anymore — the engine owns both.
    3. The pushed tag triggers `release.yml` (cargo-dist): builds the cross-platform matrix +
       creates/finalizes the GitHub Release. **macOS aarch64 builds on the personal `hauis`
       self-hosted runner** — if it 400s, clear hauis's stale token
       (`ssh hauis 'git config --global --unset-all "http.https://github.com/.extraheader"'`)
       and `gh run rerun <release-run-id> --failed`. (Coupling tracked:
       `release-macos-hauis-coupling`.)
    4. Post-cut, verify the delegated Release exists: `gh release view vX.Y.Z` (until
       `release-verify-delegated-github-release` automates it into `ossctl release verify`).
  - **Fallback (manual recipe), if an engine cut fails partway:** the old hand-driven path
    still works — `gh workflow run publish-crates.yml` for crates, a hand `brew bump-formula-pr`
    for the tap — and `ossctl release resume <run>` / `abandon <run>` recover an interrupted
    engine run. The three pre-engine defects (`publish-crates-no-auto-trigger`,
    `homebrew-tap-bump-manual-and-missed`, `release-macos-hauis-coupling`) are now mostly
    subsumed by the engine cut; hauis coupling is the surviving CI-side item.
- **Git: `pull --rebase` → `push` is always allowed, no confirmation** (maintainer
  decision, 2026-08-05). On this repo the agent may run the pull-rebase-push sequence
  (`git pull --rebase origin main` then `git push origin main`, and pushing tags) on its own
  whenever `main` is clean and green — publishing commits to the remote does not need a
  separate go. (This is a repo-scoped grant that overrides the global "pushing is the user's
  step" default.) Still: never force-push a shared branch, and never push a red tree.
- **Scope boundary: ossctl the PRODUCT ≠ Jari's personal environment.** ossctl owns the
  generic, reusable release/readiness engine (e.g. `ossctl dist generate`). It does **not**
  own the cross-repo *standardisation* of Jari's other CLIs (issuectl/orchestratectl/glasspad)
  or the personal self-hosted CI infra (the `hauis` runners) — those are **homebase**
  concerns (homebase issue `cross-repo-release-standardisation`). Keep that work out of
  ossctl's issues/TODO/handoff; only the generic capability lives here, its downstream use
  across Jari's repos is a homebase matter. (The `hauis` override in `dist-workspace.toml`
  is a documented personal exception for ossctl's own build, tracked for decoupling in
  `release-macos-hauis-coupling`.)
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
