# ossctl

Release & readiness coordinator: the deterministic engine that takes any repo to
OSS release quality — the AI-first Rust CLI behind the `/oss-*` Claude Code skill
family. `ossctl` owns the normalizer/validator for the project release contract
(`OSS-RELEASE.md`), repo-fact detection, the readiness audit, and the resumable
per-ecosystem release-cut state machine; the prose `/oss-*` skills are thin callers
of this binary (the binary is the source of truth, §17).

**Status: public, live.** Current release: see `ossctl version --json` / the git tags.
Shipped on all four channels — crates.io (`ossctl` + `ossctl-core`), GitHub Releases
(cargo-dist: macOS aarch64, Linux musl x86_64+aarch64, `.sh` installer), and the
Homebrew tap. **No Windows** (maintainer decision 2026-08-17; deliberate, documented in
`DEFAULT_CROSS_PLATFORM_TARGETS`). Version history: `CHANGELOG.md` + git tags.

## CLI Design Principles

Use the `/ai-first-cli-canon` skill shipped by `project-canon` as the maintained AI-first
CLI canon. It is the binding reference for CLI surface work: strict input validation,
`--json` output, JSONL logs, no interactive prompts, informative errors and composable
commands. Do not keep or edit a repo-local `AGENTS-AI-FIRST-CLI.md` copy; update the
canon at its source and reinstall the skill from the released tool.

## Architecture (decided before code — read first)

The architecture lives in accepted ADRs under [`docs/adr/`](docs/adr/) — read them before
writing any code; they are the spec, not background:

- [`0001-founding-architecture.md`](docs/adr/0001-founding-architecture.md) — CLI command
  taxonomy, the two-crate workspace (`ossctl-core` lib + the `ossctl` bin crate in
  `crates/ossctl-cli/`), the binary↔skill boundary.
- [`0002-release-engine-adapter-model.md`](docs/adr/0002-release-engine-adapter-model.md)
  — the `ReleaseAdapter` trait + enum registry, the phase-barrier coordinator, the sealed
  content-addressed `plan_id` approval seam, and (amendments) the cargo interleave and the
  mandatory post-cut **verify** barrier ("a publish target that cannot be observed after
  the fact is not a publish target"; `Unknown` is not green).
- [`0003-config-and-journal-storage.md`](docs/adr/0003-config-and-journal-storage.md) —
  `OSS-RELEASE.md` as the project contract; the event-sourced JSONL release journal under
  `git-common-dir/ossctl/releases/<run_id>/`; the remote-is-ground-truth resume/reconcile
  table; and (amendment) the durable **plan store** under `git-common-dir/ossctl/plans/`.
- [`0004-cargo-adapter-one-target-one-publish-unit.md`](docs/adr/0004-cargo-adapter-one-target-one-publish-unit.md)
  — one target = one publish unit; the coordinator owns cross-target ordering.

Open an `issuectl` issue before building a feature — do not pre-design beyond the ADRs.

## Operating policy (for `/stint`)

`/stint` reads this section for how to run a work-session in this repo.

- **Green gate** (must pass before a unit counts as landed). The repository's
  `rust-toolchain.toml` pins the same Rust release used by CI; do not override it with an
  ambient `stable`, because Clippy lint sets change between releases. Tests in this gate
  must be hermetic: an unavailable destination is `Unknown`, but fixtures must inject
  destination responses rather than depend on host credentials or network access.
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo build --workspace` (release build not required per-unit)
  - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — CI runs this and it is
    easy to miss locally: broken intra-doc links fail the `docs` job even when tests pass.
- **Releases MAY be cut automatically whenever there is something to release** (maintainer
  decision, 2026-08-05), and **the engine-driven cut is fully autonomous — NO go/no-go
  checkpoint, ever** (2026-08-06). Do not stop to ask "shall I cut?" — run the recipe end
  to end and report as you go. The safety is structural: the sealed plan, `dry-run-all`
  before any publish, dep-order + index-wait on crates.io, the `undeclared_distribution`
  refusal, `resume`/`abandon`, and the mandatory **verify** phase (green means every
  target was OBSERVED at its destination). Still: green gate first, plan first, never
  publish red, report each phase. Trust the refusals — and since the verify phase, the
  green is observation-backed, not assumed.
- **The ENGINE recipe** (`ossctl release cut` — the primary and proven path; ossctl's own
  contract declares all four targets + the `distribution` block with its tap):
  1. Ensure `CHANGELOG.md` `[Unreleased]` is complete and main is clean + pushed.
  2. **Build a fresh binary from the tree**: `cargo build --release -p ossctl` (the bin
     crate is **`ossctl`**, NOT `ossctl-cli` — `-p ossctl-cli` silently no-ops). `plan`
     and `cut` refuse when the binary's compiled commit differs from tree `HEAD`
     (`--allow-stale-binary` is the escape hatch for deliberate cross-tree use).
  3. `ossctl release plan --bump major|minor|patch` — seals the plan (bump version,
     `=`-pin rewrites, CHANGELOG finalize plan) and persists it in the plan store.
     Inspect the JSON. There is **no `--version` flag** (version derives from the
     manifest; `--bump` computes the next one).
  4. `ossctl release cut --plan <id>` — the `--bump` flag is optional (the cut recovers
     the bump disposition from the stored plan). Phases: `bump → dry-run-all → build-all
     → publish-all (crates.io, dep-ordered, index-waited) → tag (GitHub Release delegated
     to cargo-dist CI) → dist (engine tap-write, real per-platform sha256s) → verify
     (all targets observed: index, Release assets, tap formula)`.
  5. Post-cut: fast-forward local `main` to the bump commit and push it (the tag push
     carries the commit to the remote, but not the branch ref). A manual spot-check of
     the channels is optional — the verify phase already observed them.
  - **hauis note:** macOS aarch64 CI builds run on the personal `hauis` self-hosted
    runner. If it 400s:
    `ssh hauis 'git config --global --unset-all "http.https://github.com/.extraheader"'`
    then `gh run rerun <run-id> --failed`. (Tracked: `release-macos-hauis-coupling`.)
  - **Resume semantics:** `release resume <run>` executes the run's **stored sealed
    plan** against a clean checkout of the sealed commit, so it survives a code fix
    moving HEAD (plan-store era). `resume --allow-unverified` skips only targets whose
    verify is `Unknown` — never `Missing`/`Conflicts`. `release abandon <run>` is the
    exit for runs that should not finish, and it can break a provably-dead holder's
    stale lock on its own.
  - **Fallback (partial-failure insurance):** the manual path still works —
    `gh workflow run publish-crates.yml` for crates, a hand formula bump for the tap.
- **Three publish dispositions — one vocabulary** (0.8.0/0.9.0). Every target declares who
  performs the publish, and the engine's job differs accordingly:
  - **Engine publishes** — `cargo-publish` (crates.io), `homebrew-tap` (engine writes the
    formula, carrying the first-line marker `# Generated by ossctl; do not edit by hand`).
  - **CI publishes, engine OBSERVES** — `cargo-publish-ci` (crates.io; the cut stops at the
    pushed tag and verify watches the index), `cargo-dist` (gh-releases, and homebrew when
    cargo-dist's own `publish-jobs` owns the tap). The engine writes nothing and requires no
    marker, but verify still polls the destination with a bounded wait
    (`DELEGATED_RELEASE_VERIFY_TIMEOUT_SECS`, 20 min / 15 s). Delegated ≠ unverified.
  - **Nothing is published** — an authored `targets: []` survives normalization as a
    tag-only cut: publishes nothing, delegates no Release, and passes verify *vacuously*
    (nothing to observe, which is categorically NOT `Unknown`). A `distribution:` block
    next to `targets: []` is refused, so a tag-only cut cannot trigger cargo-dist behind
    the plan's back.
- **Homebrew tap ownership — ossctl is the exception, not the pattern.** ossctl owns its
  OWN tap via `homebrew-tap` (its `dist-workspace.toml` has NO `publish-jobs`), and it is
  the only live exercise of that adapter. Every other fleet repo
  (issuectl / glasspad / orchestratectl / project-canon) carries
  `publish-jobs = ["homebrew"]`, so cargo-dist's `publish-homebrew-formula` job writes
  their formula on every tag: their contracts **must declare the homebrew target with
  `adapter: cargo-dist`**. Declaring `homebrew-tap` there creates a double writer (this
  already false-red'd an issuectl cut on a transient 503); omitting the target entirely
  under-declares a channel users install from. Full fleet picture:
  `homebase/issues/cross-repo-release-standardisation/audit-2026-08-17.md`.
- **`SEAL_VERSION` is 6.** The sealed plan's pre-image includes the phase sequence, so a
  phase-model change is a deliberate `SEAL_VERSION` event per `release/plan.rs`'s evolution
  rule — never a silent hash change. Live consequence of the 5→6 bump (0.8.0, when `verify`
  joined the phase list): a plan sealed by an older binary can no longer be cut and must be
  re-planned. Legacy plans still *load*, so an interrupted run can still `resume`.
- **Git: `pull --rebase` → `push` is always allowed, no confirmation** (maintainer
  decision, 2026-08-05), including tag pushes, whenever `main` is clean and green. This
  repo-scoped grant overrides the global "pushing is the user's step" default. Never
  force-push a shared branch; never push a red tree.
- **Scope boundary: ossctl the PRODUCT ≠ a maintainer's personal environment.** ossctl
  owns the generic, reusable release/readiness engine. Cross-repository standardisation
  and personal self-hosted CI infrastructure are homebase concerns — keep them out of
  ossctl's issues/TODO/handoff. The `hauis` override in `dist-workspace.toml` is the
  documented repo-local exception.
- **Cross-platform is a hard requirement (macOS AND Linux, arm64 + x86_64).** Every tool
  the `/oss-*` family produces — and ossctl itself — must offer a source path
  (`cargo install`) plus prebuilt binaries/installers covering macOS and statically-linked
  musl Linux. A macOS-only or Linux-only install story is a release gap.
- **No Code of Conduct — deliberate** (maintainer decision). `ossctl audit` listing it as
  a `recommended` gap is expected and accepted; do not propose adding one.
- **Live-version check:** `ossctl version --json`.
- **Hot files.** Two classes — do not treat them the same:
  - **Append-union-safe — parallel is fine:** `Cargo.toml`, module `mod.rs` files, CLI
    subcommand-dispatch files, the bundled-skill `CATALOG` in
    `crates/ossctl-cli/src/skill.rs`. Brief each worker to union-resolve (keep all deps /
    decls / arms / rows). The auto-merge is not guaranteed, though — expect to salvage
    the last-in-line row-adder's merge by hand occasionally.
  - **True shared-logic — sequence strictly, never parallelise:**
    `crates/ossctl-core/src/contract/schema.rs` (the ONE canonical serde model), any
    existing shared `crates/ossctl-core/src/protocol/*.rs` module (a NEW file per unit is
    append-safe), `crates/ossctl-core/src/release/coordinator.rs` +
    `crates/ossctl-core/src/release/adapters/mod.rs` (the release-engine seam), and the
    canonical-JSON contract shape (SCHEMA — ripples to every family member).
- **Worker-model note:** for units on the coordinator/adapters seam, prefer the stronger
  worker model up front — a weaker model has twice abandoned mid-unit there.
- **Migration rule:** the canonical-JSON output shape is a schema-versioned compatibility
  contract (§10). Preserve it; bump `schema_version` on a breaking change, never silently.
- **Test-account reset:** n/a (no external test accounts).
- **Issue standard: a finding earns a place in the tracker only if its failure can
  actually occur here** (maintainer decision, 2026-08-17). Judge the content: is the
  failure reachable in this project, on this path, and what is the damage beyond an error
  message? Provenance (a review panel, several models agreeing) is a supporting signal,
  never the verdict — models correlate hardest on plausible-sounding generic advice.
  **Reject** cosmic-ray scenarios, duplicate checks, and hostile-input hardening where the
  only actor is the maintainer's own machine. **Keep** an unobserved finding when the
  failure would be silent, irreversible, reachable by a downstream user, or contradicts a
  documented guarantee. When closing one, record the reason **and a reopen condition**.
  Also applies to **deferral justifications** — verify a claimed blocker, never inherit it.

## Companion-skill installer (`ossctl skill install`)

The bundled `/oss-*` skills install into a per-runtime skills home. `skill install`
**dual-homes** by default — with no `--agent`, each `SKILL.md` is written into both
`~/.claude/skills/<name>/` (Claude Code) and `~/.pi/agent/skills/<name>/` (pi.dev).
`--agent` narrows it: `claude` | `pi` | `codex` (flat `~/.codex/prompts/<name>.md`) |
`all`. Only `SKILL.md` is mirrored; the install is idempotent, §17 version-guarded, and
the write is atomic. `--dest <PATH>` overrides the root.

## Gitignored directories

- `history/` — agent scratchpad and ephemeral planning docs (not tracked)

## Documentation Pattern

Every directory follows this structure:

- `CLAUDE.md` — symlink to `AGENTS.md`
- `AGENTS.md` — all AI-relevant info (consolidated)
- `AGENTS-<TOPIC>.md` — complex topics split out (optional)

## Issues & Planning

Issue tracking is managed by `issuectl`. Use the `/issue` skill (installed by
`issuectl init`) to create, search, update, and close issues.

- `issues/<slug>/item.md` — every issue and epic (flat layout)
- Status lives in the `status:` frontmatter field, not in the path
- `issues/AGENTS.md` — issue schema, types, workflow (owned by issuectl)
- `.issuectl/AGENTS.md` — repo-local policy for AI agents (owned by issuectl)

All planning documents (plans, analyses, validations, designs, breakdowns, todos) belong
under their parent issue directory — not as standalone files. If work needs a planning
document, it also needs an issue.
