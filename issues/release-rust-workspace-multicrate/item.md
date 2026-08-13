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
