# ossctl

Release & readiness coordinator: the deterministic engine that takes any repo to
OSS release quality — the AI-first Rust CLI behind the `/oss-*` Claude Code skill
family. `ossctl` owns the normalizer/validator for the project release contract
(`OSS-RELEASE.md`), repo-fact detection, the readiness audit, and the resumable
per-ecosystem release-cut state machine; the prose `/oss-*` skills are thin callers
of this binary (the binary is the source of truth, §17).

## CLI Design Principles

Use the `/ai-first-cli-canon` skill shipped by `project-canon` as the maintained AI-first CLI canon. It is the binding reference for CLI surface work: strict input validation, `--json` output, JSONL logs, no interactive prompts, informative errors and composable commands. Do not keep or edit a repo-local `AGENTS-AI-FIRST-CLI.md` copy; update the canon in `~/Sources/project-canon` and reinstall the skill from the released tool.


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
  - **Shipped: 0.1.0 (2026-08-04), 0.1.1 (2026-08-05), 0.1.2 (2026-08-05), 0.2.0 (2026-08-06),
    0.2.1 (2026-08-06), 0.2.2 (2026-08-06), 0.2.3 (2026-08-07), 0.2.4 (2026-08-10), 0.2.5 (2026-08-10),
    0.3.0 (2026-08-11), 0.4.0 (2026-08-11), 0.5.0 (2026-08-13).**
    All on crates.io (`ossctl` + `ossctl-core`), GitHub Releases (cross-platform: macOS aarch64,
    Linux musl x86_64+aarch64, Windows, `.sh`+`.ps1` installers), and the Homebrew tap
    `jarimustonen/homebrew-ossctl`. Repo is **public**. 0.1.2 added `ossctl dist generate`. 0.2.0
    made the engine drive a multi-target cut (multi-target/ecosystem in dep order, one-target-one-
    publish-unit/ADR-0004, CI-delegated skip, GH-Release-to-CI, post-tag homebrew, crates.io-pin)
    + `release list`/`abandon`. 0.2.1 landed the INTERLEAVE fix (adapter defers a `=`-pinned
    dependent's packaging into its dep-ordered `cargo publish`; ADR-0002 amendment) — the first cut
    to clear `dry-run-all` + `build-all` through the engine for all four targets. 0.2.2 wired the
    crates.io **RegistryQuery** for `rust` (sparse index), clearing the PUBLISH barrier. 0.2.3 made
    the homebrew dist leg **self-sufficient** (direct tap-write, dropping the `brew bump-formula-pr`/
    `brew audit` dependency) + unified the registry probes behind a `ureq` `http_get` seam. 0.2.5
    hardened the real-cut publish: a **self-visibility confirm** (the adapter verifies the target's own
    `{name,version}` reached the index — reusing the bounded index-wait — before journaling a receipt,
    so a silent no-op upload fails the cut closed instead of fabricating a receipt) + made the release
    version **single-source** (derived from the workspace manifest; `--version` is an optional must-match
    confirmation) + made the generated/own `publish-crates.yml` idempotent on "already exists". 0.3.0
    (BREAKING) **removed `--version`** entirely (version derives solely from the manifest; a stray flag is
    now a hard error — see recipe) + made the version guards **fail-closed for manifest-versioned non-Rust**
    ecosystems (was fail-open for node/python) + made `cut`/`resume` publish from a **clean checkout of the
    sealed `head_sha`** (reproducible, immune to mid-cut edits) + **digest-authenticated** the resume
    idempotency skip (checksum-match the on-registry crate before trusting a skip, else fail closed). 0.4.0
    made `ossctl skill install` **dual-home into pi.dev**: a new `pi` runtime writes each SKILL.md into
    `~/.pi/agent/skills/<name>/` and — with `--agent` omitted — the installer now writes BOTH Claude and
    pi.dev by DEFAULT (`--agent claude` restores single-home; `pi`/`codex` narrow; `all` = every runtime).
    0.5.0 completed **`release-rust-workspace-multicrate`** (retires hand-cut downstream releases): the plan
    now derives the dep-ordered multi-crate publish **CLOSURE** from a bin-only contract (lib → bin) + carries
    `homebrew_tap` from the contract distribution block, and the engine OWNS the version bump via
    `release plan/cut --bump major|minor|patch` — a cut-time executor sets the workspace version, rewrites the
    `=`-lockstep pins, refreshes Cargo.lock, finalizes the CHANGELOG, runs a contract-declared
    `release.bump_hook`, commits, and tags the bump commit (resume-safe; journal v3→v4). The `--bump` live
    acceptance is a downstream cut of **orchestratectl 0.2.0** (prepared 2026-08-14: its contract now declares
    the `bump_hook` + a `distribution` block; runs on orx's timeline).
  - **🎉 THE DOGFOOD IS COMPLETE (stint #14).** `ossctl release cut` now cuts ossctl ITSELF
    end-to-end, fully autonomously, with zero manual publish steps — proven by the **0.2.3 cut
    (2026-08-07)**: dry-run-all → build-all → publish-all (both crates → crates.io) → tag → dist,
    where the dist phase published the HOMEBREW leg itself via the direct tap-write. Exit 0,
    "published 4 target(s)". History: 0.2.0's two engine cuts failed SAFELY at BUILD (fixed by the
    interleave, stint #13); the 0.2.1 cut failed SAFELY at PUBLISH (no crates.io RegistryQuery for
    `rust`, fixed by `release-publish-registry-query-not-wired`, 0.2.2); the 0.2.2 cut failed SAFELY
    at DIST on the homebrew `brew audit` (fixed by `homebrew-dist-brew-audit-fails`, 0.2.3). Each
    blocker fell in turn, always failing closed/safe. **0.2.0 and 0.2.1 were cut manually; 0.2.2 was
    cut by the engine except its homebrew leg (done by hand); 0.2.3 was fully engine-cut.** No HIGH
    blocker remains.
  - **The ENGINE recipe (`ossctl release cut`) is the PRIMARY and PROVEN path** — the 4-step manual
    fallback is RETIRED for ossctl's own cut (kept below only as partial-failure insurance). ossctl's
    own `OSS-RELEASE.md` declares the
    four targets (ossctl-core + ossctl on crates.io; ossctl on gh-releases/cargo-dist; ossctl
    on homebrew) plus a `distribution` block with `homebrew_tap: jarimustonen/homebrew-ossctl`.
    The recipe:
    1. Bump `workspace.package.version` + the internal `=X.Y.Z` dep in lockstep → finalize
       CHANGELOG → `cargo build` (refresh lock) → `cargo publish -p ossctl-core --dry-run` →
       commit → push `main`.
    2. **Cut with a FRESHLY-BUILT binary from the current tree.** Build it as
       `cargo build --release -p ossctl` (the bin crate is **`ossctl`**, NOT `ossctl-cli` —
       `-p ossctl-cli` silently no-ops and leaves a STALE binary) and verify `./target/release/ossctl
       version` prints the just-bumped version. ⚠️ Since 0.2.5 the version is read from the tree at
       runtime, so `release plan` shows the NEW version even from a stale old binary — `plan`/`cut`
       will then run OLD engine code silently. `ossctl version` is the only tell. (Stale-binary guard
       tracked in `release-cut-stale-binary-guard`.) Then
       `ossctl release plan` (seal + inspect; side-effect-free) → then
       `ossctl release cut --plan <id>`. **There is no `--version` flag** (removed in 0.3.0,
       `release-drop-version-flag`): the release version comes solely from the workspace manifest
       (the version you bumped in step 1), so a stray `--version` is now a hard clap error, not a
       silently-ignored confirmation. The engine runs
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

## Companion-skill installer (`ossctl skill install`)

The bundled `/oss-*` skills install into a per-runtime skills home. `skill install`
**dual-homes** by default — with **no** `--agent`, each `SKILL.md` is written into
**both** `~/.claude/skills/<name>/` (Claude Code) **and** `~/.pi/agent/skills/<name>/`
(pi.dev), so a skill is discoverable under either harness (pi.dev resolves it as
`/skill:<name>`; bare `/name` cross-references also resolve via pi's injected
available-skills list, so only the install *target* changes — no cross-reference
rewrite). This is the migration default (agents moving Claude Code → pi.dev).

`--agent` narrows it: `claude` → `~/.claude/skills` only, `pi` → `~/.pi/agent/skills`
only, `codex` → `~/.codex/prompts/<name>.md` (flat), `all` → every known runtime
(Claude + pi.dev + Codex). Claude and pi.dev share the directory-per-skill shape
(`<name>/SKILL.md`); Codex uses a flat prompt file. Only `SKILL.md` is ever mirrored
(ossctl's bundled skills are single-file), so the vendored-filtering is inherent. The
install is idempotent and §17 version-guarded, the write is atomic (a `rename`
replaces a *final-component* symlink rather than following it — POSIX; ancestor dirs
are not no-follow), and the `--json` envelope reports one `installed[]` row per
requested target — the object shape is unchanged (additive), though the omitted-flag
**default changed** from Claude-only to Claude + pi.dev, so it now writes two targets
and emits two rows where it wrote one. `--dest <PATH>` overrides the root;
shape-sharing runtimes rooted at the same `--dest` (Claude + pi.dev) resolve to one
file, so the *write* collapses to a single file while both runtimes still get their
report row.

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
