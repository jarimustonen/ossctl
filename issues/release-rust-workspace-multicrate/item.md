---
created: 2026-08-12
updated: 2026-08-13
type: feature
status: in-progress
priority: high
labels: [release, rust]
commits:
- hash: 8a0b319
  summary: derive dep-ordered multi-crate workspace publish set in the plan (facet 1+4)
- hash: '604e092'
  summary: publish dependency closure + precise workspace edges (llm-review fixes)
- hash: 8a25b69
  summary: engine-owned --bump plan phase + contract bump_hook (facets 2+3, plan side; cut fails closed pending executor)
- hash: '8276630'
  summary: apply llm-review consensus fixes (build_with_bump owns arithmetic, preview warning, hook security note, changelog match)
- hash: ce33e25
  summary: cut-time bump executor (facets 2/3 executor half); green + tested; issue stays open pending live acceptance cut
---

# release engine: support dependency-ordered multi-crate Rust workspace publish + version bump (retire hand-cut releases)

## Description

GOAL: make `ossctl release plan/cut` able to cut orchestratectl's releases so we can RETIRE hand-cutting (0.1.1-0.1.6 were all hand-cut; `ossctl release list` shows only an abandoned v0.1.0 run — the engine has never successfully cut a release for this repo).

## Observed gap (ossctl 0.2.2, repo ~/Sources/orchestratectl)
`ossctl release plan --version 0.1.6 --json` for a two-crate workspace (crates/octl-core lib + crates/octl-cli bin `orchestratectl`, where the CLI depends on `octl-core = "=<version>"`) produced an INCOMPLETE plan:
- targets: **only** `{ecosystem: rust, package: orchestratectl, registry: crates.io}` — `octl-core` is NOT a target, so a cut would `cargo publish orchestratectl` while `octl-core@<new-version>` does not yet exist on crates.io → publish fails on the `=<version>` pin.
- **no version-bump phase**: head_sha was the pre-bump commit (Cargo.toml still at the old version); phases were [dry-run-all, build-all, publish-all, tag, dist]. Nothing bumps the workspace `version`, the octl-cli `octl-core = "=X"` pin, or Cargo.lock.
- `homebrew_tap: null` even though OSS-RELEASE.md's distribution + the per-tool tap `jarimustonen/orchestratectl` exist.

## What the engine should do for a Rust workspace release
1. **Derive dependency-ordered member publish** from the workspace graph: publish path-dependency crates before their dependents (octl-core → orchestratectl), waiting for each to be available on the registry (as `cargo publish` already does within a crate). Both members are `publish = true`.
2. **Own the version bump** as a plan phase: set the workspace `[workspace.package] version`, rewrite intra-workspace `=<version>` pins (octl-cli's `octl-core = "=X"`), refresh Cargo.lock, and finalize CHANGELOG (`[Unreleased]` → dated) — content-addressed like the rest of the plan.
3. **Regenerate version-embedding test snapshots** as part of the bump (orchestratectl has insta `envelope_snapshots__version_{text,json,jsonl}` that embed the version + per-skill cli_version; a bump restales them and reds CI). Either regenerate + strip insta's volatile `assertion_line:` header, or provide a documented hook the repo runs.
4. **Carry homebrew_tap** from the contract's distribution block into the plan (tag → cargo-dist Release CI builds binaries + updates the tap).

## Evidence
`ossctl release list --json` → only abandoned v0.1.0. `ossctl release plan --version 0.1.6 --json` → the single-target/no-bump plan above. Hand-cut reference (what the engine must reproduce): the `release: vX.Y.Z` commits on orchestratectl main + TODO.md 'RELEASE STATE'.

## Done
`ossctl release cut` produces a correct, coherent orchestratectl release end-to-end (both crates published in order at the bumped version, snapshots green, tag pushed, tap updated) — verified on a real cut — so the repo's AGENTS.md 'the /oss-release skill orchestrates the whole thing' becomes true and hand-cutting is retired.

---

## Progress — spinoff (2026-08-12): facets 1 + 4 landed; 2 + 3 need a maintainer decision

**Landed (green, multi-model reviewed):**

- **Facet 1 — dependency-ordered member derivation. DONE.** `ossctl release plan`
  now derives the full, dependency-ordered publish set for a multi-crate Cargo
  workspace from the workspace graph, so a contract that declares ONLY the bin crate
  yields BOTH crates as ordered publish units (lib before bin). Implemented as an
  off-wire `Facts::rust_workspace` graph (`detect_rust_workspace`, precise
  path/workspace edge parsing) + `expand_rust_workspace_members` in the plan.
  **Publishes the declared targets' TRANSITIVE dependency CLOSURE, not every
  publishable member** (an unrelated, deliberately-undeclared crate is never swept in
  — an llm-review safety fix). ossctl's own plan is unchanged (strict superset). Proven
  end-to-end: `release plan --json` on a two-crate fixture declaring only the bin
  yields `[octl-core, orchestratectl]` (crates.io) and excludes an `experimental` crate.
- **Facet 4 — homebrew_tap carry. DONE (was already carried; now tested).** The plan
  threads the per-tool tap from the contract's distribution block; verified non-null for
  the orchestratectl fixture (`jarimustonen/orchestratectl`).

**NOT landed — need a maintainer design decision (left open deliberately):**

- **Facet 2 — engine-owned version bump.** As specified ("plan at the PRE-bump commit;
  engine sets `[workspace.package] version`, rewrites `=X` pins, refreshes Cargo.lock,
  finalizes CHANGELOG"), this CONTRADICTS the current single-source-version model:
  since `--version` was removed in 0.3.0, the release version derives SOLELY from the
  manifest. There is no channel to tell the engine "bump to 0.1.6" when planning at the
  pre-bump commit — the manifest still reads the old version. Owning the bump therefore
  requires re-introducing a version INPUT (a `--version`/`--bump major|minor|patch`
  flag, or reading an intended version from CHANGELOG `[Unreleased]`), which re-opens a
  settled architecture decision. **Decision needed:** where does the new version come
  from when the engine owns the bump? Once decided, the derived edits (pin rewrites +
  lockfile refresh + CHANGELOG finalize) are a tractable content-addressed plan phase.
- **Facet 3 — snapshot regeneration.** Depends on facet 2 (a bump is what restales the
  snapshots). Open design choice: an in-engine regen step (run the repo's test-update
  command, strip insta's volatile `assertion_line:` header) vs. a contract-declared
  `bump_hook` the repo runs. Recommend the contract-declared hook (keeps the engine out
  of arbitrary test-harness specifics); defer until facet 2's shape is fixed.

**Ultimate acceptance (a real orchestratectl cut) is the maintainer's step** — it needs
crates.io creds + an irreversible publish and was explicitly out of this spinoff's scope.

### llm-review follow-ups (parser hardening; each fails a cut CLOSED today, never mis-publishes)

From the 4-model review (report: `history/review-release-rust-workspace-multicrate.md`).
Deferred as they are exotic for the target lib+bin shape and fail-safe:

- workspace-inherited dependency RENAMES (`alias.workspace = true` where root
  `[workspace.dependencies]` maps `alias` → a differently-named crate) drop the edge;
- `publish` field workspace inheritance (`publish.workspace = true`) not resolved;
- a NON-virtual workspace root (a root `[package]` that is also a member) is not graphed;
- multi-line inline dependency tables read only their first physical line;
- recursive/patterned member globs (`crates/**`, patterned `exclude`) unsupported
  (pre-existing facts limitation);
- Windows absolute-path / backslash member confinement (pre-existing);
- `MANIFEST_LIMIT` truncation is treated as authoritative for release parsing;
- a cut-time cross-check of the sealed plan ORDER against `cargo metadata` topology
  (the plan hash covers the target SET + order, but nothing re-validates order at cut).

## Maintainer decision (2026-08-13, stint #19 feedback round) — facet-2 version source

Facet 2's version input = **`--bump major|minor|patch`**. The engine COMPUTES the new
version from the current manifest version + the bump level; there is NO hand-typed literal
version (honors the 0.3.0 single-source-version decision — `--version` stays removed). Given
the bump level, the engine owns the derived edits as a content-addressed plan phase: set
`[workspace.package] version`, rewrite intra-workspace `=<version>` pins, refresh Cargo.lock,
finalize CHANGELOG (`[Unreleased]` → dated). Facet 3 (version-embedding snapshot regen) rides
on this via a **contract-declared bump_hook** the repo runs (keeps the engine out of per-repo
test-harness specifics), per the spinoff's recommendation.

## Progress — facets 2+3 spinoff (2026-08-13): PLAN side landed; cut-time EXECUTION deferred

**Landed (green + 4-model reviewed, `wt/01kzx1581x-release-bump-phase`):**

- **Facet 2 (plan side) — engine-owned `--bump`. DONE.** `ossctl release plan --bump
  major|minor|patch` computes the new version from the current manifest version + the
  level (strict semver, fails closed on non-semver) and seals a **content-addressed bump
  phase** (`PlanPhase::Bump` prepended): computed `to_version`, intra-workspace `=`-pin
  rewrites (derived from the workspace graph), CHANGELOG-finalize intent, and the declared
  `bump_hook`. The core constructor OWNS the arithmetic so a plan can never seal a version
  that contradicts its level. `--bump`-less plans are **byte-identical** (plan_id unchanged;
  additive/skip-none; no SEAL_VERSION bump — proven by the unchanged golden vectors).
- **Facet 3 (contract) — `release.bump_hook`. DONE.** Optional additive field (no
  `schema_version` bump — the codebase is Serialize-only, no deser hazard; matches the
  additive-field migration rule). Carried into the bump plan; surfaced verbatim as a
  plan-time reviewer warning (supply-chain eyes-on).
- **`--bump` on `plan` + `cut`** (strict enum validation, JSON error envelope); shared
  `derive_release_plan`. Unit + integration tests: version compute per level, derived edit
  set, pin rewrite, changelog finalize, hook wiring, bad-value rejection, content-addressing,
  opt-in superset. Green gate + `/llm-review` (report `history/review-release-bump-phase.md`).

**NOT landed — cut-time EXECUTION of the bump phase (the remainder; issue stays OPEN):**
`release cut` **fails closed** (`bump_execution_unimplemented`) on any bump plan rather than
build/publish the un-bumped version. The executor half could not be validated here because it
requires a real irreversible crates.io cut (the maintainer's acceptance step, explicitly out
of this spinoff's scope). Remaining work, all gated behind that live validation:

1. **Apply the sealed edits at cut time** in the clean checkout (`release-cut-clean-checkout`):
   set `[workspace.package] version`, apply the pin rewrites, `cargo update` (lockfile),
   finalize the CHANGELOG (dated), run the `bump_hook`, commit — BEFORE the build barrier —
   and point the tag at the **bump commit** (today `tag_phase` tags the pre-bump `head_sha`),
   pushing the commit to the branch.
2. **Pin-rewrite precision** (llm-review, all 4): extend `Facts` to carry each intra-workspace
   dependency's **requirement string**, and emit a `PinRewrite` only when the current req
   literally equals `=<from>` (today it assumes the `=`-lockstep convention for every
   member→member edge — correct for the lib+bin target, over-broad for caret/range/
   `workspace = true`/independent-version members). The executor must verify the exact old
   value before replacing and fail closed on zero/multiple matches.
3. **Post-bump resume/verify state machine**: `BumpState { NotStarted | Applied{bump_commit,
   effective_date, tree_hash} }` on the journal; `verify` must re-derive the bump (not hold
   `approved.bump` fixed); resume must recognize the recorded bump commit and never re-bump.
   Journal the effective changelog **date** once and reuse on resume (today it is unsealed).
4. **`bump_hook` execution contract**: shell-vs-argv, working dir, timeout, environment/secret
   policy, permitted file changes, and a post-hook validation pass (re-read manifests + lock +
   changelog, reject unexpected modifications). Document the trust model.

Acceptance (a real orchestratectl cut through the engine) remains the maintainer's step.

## Progress — cut-time EXECUTOR landed (2026-08-13, `wt/01kzx…-release-bump-executor`)

**Landed (green gate + `/llm-review`; commit `ce33e25`):** all four remainder items above are
implemented and unit/integration-tested. `release cut`/`resume` now EXECUTE the sealed bump phase;
the `bump_execution_unimplemented` fail-closed guard is **removed**.

- **1. Cut-time apply.** New `release/bump_exec.rs` (effectful) + pure transforms in `release/bump.rs`
  (`set_workspace_version`, `rewrite_pin`, `finalize_changelog`, `workspace_version`). The coordinator's
  `bump_phase` runs FIRST (before dry-run), inside the clean checkout of the sealed head_sha: sets the
  workspace version, applies the pin rewrites, `cargo update --workspace` (lockfile), finalizes the
  CHANGELOG (dated), runs any `bump_hook`, commits, and journals `BumpApplied{commit, effective_date}`.
- **2. Tag the bump commit.** `tag_phase` now points the tag at the bump commit (from `RunState.bump.commit`),
  not the pre-bump `head_sha`. The bump commit is committed in the linked worktree (shared object store)
  and best-effort pushed to the real repo's branch (`git push origin HEAD:refs/heads/<branch>`).
- **3. Pin-rewrite precision.** `Facts::WorkspaceMember.dep_reqs` carries each intra-workspace dep's literal
  requirement string; the planner emits a `PinRewrite` ONLY for edges whose req literally equals
  `=<from_version>` (caret/range/`workspace = true` edges left untouched). The executor re-verifies the
  exact old value and fails closed on zero/multiple matches.
- **4. Resume/verify state machine.** `Phase::Bump` + `EventKind::BumpApplied` + `RunState.bump` (NotStarted
  = `None` / Applied = `Some`). `RunCreated` persists the sealed head_sha + bump inputs (level, from_version)
  so `resume` reconstructs the exact sealed plan via `build_with_bump` against the pre-bump commit (HEAD moved
  past it). The bump phase is idempotent — a journalled `BumpApplied` skips re-apply (no double-bump). The
  effective CHANGELOG date is journalled once and reused on resume. Journal schema **v3 → v4** (a v3 reader
  refuses a v4 line, per the migration rule; additive `RunState` fields are `#[serde(default)]`).
- **bump_hook execution contract** (supply-chain surface, documented in `bump_exec.rs`): `sh -c "<hook>"`
  with the hook string as a SINGLE verbatim argv element (NO interpolation of cut-time data); cwd = checkout
  root; inherits the cut's ambient environment (build.rs-level trust); post-hook validation re-reads the
  workspace manifest and rejects the cut if the hook altered the version; non-zero exit fails closed.
  **Known gap:** no per-command timeout (the `CommandRunner` port has none) — CI job timeout is the backstop.

**Golden vectors / plan_id stability preserved:** the no-bump path is byte-identical (SEAL_VERSION unchanged;
`--bump`-less plans hash as before). No golden-vector changes were needed.

**STILL PENDING — the live acceptance cut (maintainer's step).** The executor is proven end-to-end at the
coordinator/unit level against fakes + a real temp checkout (`release::coordinator::a_bump_plan_applies_…`,
`…_does_not_double_bump`, `release::bump_exec::*`), but a **real crates.io cut through the engine** (irreversible
publish) was explicitly out of scope and NOT run. Remaining validation, all gated behind that live cut:
- a real `ossctl release cut --bump` on orchestratectl (the original driver) — proves the whole flow end to end;
- the **best-effort branch push** (`HEAD:refs/heads/<branch>`) is untested against a real remote — verify it
  advances `main` as intended (or refine to a safer branch-advance);
- the `bump_hook` against orchestratectl's real insta snapshot-regen command;
- the `bump_hook` timeout gap (add a first-class per-command timeout to `CommandRunner`).

Issue stays **OPEN** until a live cut proves it.

### llm-review follow-ups (4-model review, `history/review-release-bump-executor.md`)

**Applied before merge (consensus correctness/mechanical):** the unanimous critical bug — a
resumed `--bump` run built/published the PRE-bump tree (checkout was re-materialized at
`head_sha`, bump re-apply skipped) → now the checkout is materialized at the recorded bump
commit on resume; `set_workspace_version` verifies `from_version`; lockfile refresh is skipped
when no `Cargo.lock` is tracked; the premature best-effort branch push was removed; and resume of
a `--bump` run now refuses with a clear `resume_bump_unsupported` message instead of a misleading
drift error.

**Deferred (each fails closed today for the shapes it doesn't handle; gated behind the live cut):**

1. **Hand-rolled TOML on the cut path.** Three line-oriented parsers (facts, `bump.rs`,
   `bump_exec.rs`). Robust + fail-closed for the `dep = { path, version = "=X" }` inline/sub-table
   convention this targets, but a `toml_edit` migration is the correct long-term fix (all 4 models).
2. **Pin coverage gaps → add a post-edit "no stale `=<from>` pin remains" scan.** Inherited
   (`[workspace.dependencies]` `= "=X"` + member `workspace = true`), `package =` renames,
   dotted-key `dep.version`, and a dep pinned in BOTH `[dependencies]`+`[build-dependencies]` are
   not rewritten. Most fail closed at publish, but the inherited-pin case can leave a stale root
   pin (mixed release). Richest fix: carry manifest-path + key-alias on `PinRewrite` and scan for
   any surviving `=<from>` intra-workspace pin after applying edits, failing closed. **Highest-
   priority remaining hardening** before a non-`=`-inline workspace uses `--bump`.
3. **bump_hook hardening.** Clear release secrets (`CARGO_REGISTRY_TOKEN`, …) from the hook env;
   validate more post-hook invariants than the root version; the hook is not exactly-once on a
   crash-before-journal. Needs a `CommandRunner` env/timeout extension (also the timeout gap).
4. **Durable bump-commit ref** (`refs/ossctl/bump/<run>`) to survive GC across a (future) bump-run
   resume; not needed for the single-execute happy path.
5. **Branch advancement.** The executor no longer pushes the bump commit to `origin/<branch>`, so
   after a cut the tag has the release commit but `main` does not. Add a safe, journalled
   branch-advance phase (fast-forward-required) or document that the operator fast-forwards `main`
   to the tag post-cut.
6. Lower severity: `cargo update --workspace` determinism (pin/validate the lock diff), CHANGELOG
   Keep-a-Changelog link-refs, CRLF preservation, `civil_date` UTC-vs-local, a real-git resume
   integration test, and cross-version (v1–v4) journal reduce tests.
