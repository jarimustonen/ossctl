# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-05 (stint #11). New agent: read this, then continue with a fresh
`/stint-start`. Main is clean, all pushed._

_🎉 **STINT #11 SHIPPED ossctl 0.1.2** — `ossctl dist generate`: the release engine now
GENERATES a downstream project's cross-platform release infra (`dist-workspace.toml` +
`release.yml` via `dist generate`) from the contract's `distribution` block. This is the
Track B piece that makes "release through ossctl" real (built against the current single
`Option<Distribution>` model; no schema change). Fully published:_
_- **crates.io** — `ossctl-core 0.1.2` + `ossctl 0.1.2` (dep order)._
_- **GitHub Release `v0.1.2`** — full cross-platform asset set (macOS aarch64, Linux musl
  x86_64+aarch64, Windows, `.sh`+`.ps1` installers, sha256 sums)._
_- **Homebrew tap** — formula bumped `v0.1.0`→`v0.1.2` (tap commit `e50fbe2`)._

_⚠️ **THE RELEASE PIPELINE IS NOT THE CLEAN AUTO-CHAIN THE OLD HANDOFF CLAIMED.** The 0.1.2
cut surfaced three real defects (all filed as ossctl issues — LANE C below). The ACTUAL,
CURRENT release recipe (do it this way until the issues are fixed):_
_1. Bump `workspace.package.version` + the internal `=X.Y.Z` dep in lockstep → finalize
   CHANGELOG → `cargo build` (refresh lock) → `cargo publish -p ossctl-core --dry-run` →
   commit → push main → `git tag vX.Y.Z && git push origin vX.Y.Z`._
_2. The tag triggers `release.yml` (cargo-dist) → builds the cross-platform matrix + creates
   the GitHub Release. **macOS aarch64 builds on the personal `hauis` self-hosted runner** —
   if it 400s, clear hauis's stale token:
   `ssh hauis 'git config --global --unset-all "http.https://github.com/.extraheader"'`
   then `gh run rerun <release-run-id> --failed`. (Tracked: `release-macos-hauis-coupling`.)_
_3. **crates.io does NOT auto-publish.** A GITHUB_TOKEN-created release does not cascade
   `release: published`, so `publish-crates.yml` never auto-fires. Publish manually:
   `gh workflow run publish-crates.yml` (uses the `CARGO_REGISTRY_TOKEN` secret; publishes
   core→cli in dep order). Verify on crates.io. (Tracked: `publish-crates-no-auto-trigger`.)_
_4. **Bump the Homebrew tap by hand** after the GitHub Release exists — the source formula's
   `url`+`sha256` do NOT auto-update (the tap silently served 0.1.0 through the whole 0.1.1
   lifetime). `curl -sL <src-tarball>; shasum -a 256`, then PUT `Formula/ossctl.rb` in
   `jarimustonen/homebrew-ossctl`. (Tracked: `homebrew-tap-bump-manual-and-missed`.)_

_**Operating policy (unchanged, both apply going forward — see AGENTS.md):** (1) releases may
be cut AUTONOMOUSLY when main has something to release (green gate first; `cargo publish`
dry-runs first; crates.io irreversible so never publish red); (2) `git pull --rebase` → `push`
is always allowed autonomously on this repo. Green gate includes
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`._

_**⭐ SCOPE CLEANUP THIS STINT — the cross-repo/personal-infra work is GONE from ossctl.**
The old handoff conflated ossctl's product backlog with Jari's personal environment. The
"multi-repo Track A rollout" (standardising issuectl/orchestratectl/glasspad) and the whole
hauis self-hosted-runner setup are **homebase concerns**, now tracked in homebase issue
[`cross-repo-release-standardisation`] (homebase commit `9c6069f`, not yet pushed). ossctl
no longer references any of it. ossctl the product only owns the GENERIC capability
(`ossctl dist generate`, shipped) — its downstream USE across Jari's repos is homebase's job._

_**ossctl's own next objective is Track B toward an engine-driven 0.2.0** (LANE A): the engine
CAN now generate config; the remaining gap is making `ossctl release cut` actually drive the
cut safely (`release-engine-cut-cargo-dist-flow` — skip CI-delegated targets + post-tag
homebrew) + the two engine bugs. A strong parallel candidate is LANE C — hardening the
fragile release pipeline itself (the 3 defects above), which would also make future cuts
one-command instead of the 4-step manual recipe._

_**Stint #11 issue work:** `release-engine-dist-config-generator` landed, closed `fixed`
(4-model /llm-review applied; green gate passed). Filed: `publish-crates-no-auto-trigger`,
`release-macos-hauis-coupling`, `homebrew-tap-bump-manual-and-missed` (LANE C)._

_--- older history in git: stints #1–7 built the `/oss-*` deterministic core
(contract/facts/audit/skill/release-engine + 9 bundled skills), #8 finished the adapters,
#9 shipped 0.1.0 by hand, #10 shipped 0.1.1 cross-platform + set up the hauis runners (now a
homebase concern). Epic `ossctl-phase4-build` stays OPEN (tails: `migrate-oss-init` deferred
until the homebase `/oss-init` copy is removed; non-Rust adapter build-side skeletons). ---_

**Read first (the spec):** `docs/adr/000{1,2,3}-*.md` (CLI taxonomy, release engine, config+journal).

## Execution DAG (2026-08-05, stint #11)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/handoff (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

The cross-repo standardisation ("Track A") and hauis CI runners are HOMEBASE concerns (Jari's
personal environment), NOT ossctl work — moved to homebase issue `cross-repo-release-standardisation`.
Do not re-add them here.

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: release-engine-cut-cargo-dist-flow   (Track B — engine-driven 0.2.0 enabler; but LANE C release-pipeline hardening is a strong parallel candidate)
LANE A — release engine (crates/ossctl-core/src/release/**; SEQUENCE strictly) — Track B toward 0.2.0
  ▶ release-engine-cut-cargo-dist-flow   (skip CI-delegated targets + post-tag homebrew — makes `ossctl release cut` real; Track B)
    release-cut-multi-target-ecosystem   (bug — >1 target/ecosystem rejected; blocks engine cut of ossctl's own 2-crate contract)
    release-list-abandon-not-implemented (bug — `release list`/`abandon` unimplemented)
    cargo-per-member-receipts        (per-member publish receipts for multi-crate cuts)
    plan-preimage-projection          (release/plan: hash a release-relevant projection, not the whole Contract)
    seal-verify-drift-dx              (release/plan: ergonomic SEAL_VERSION bump + golden-vector regen)
    homebrew-adapter-fs-port          (EffectCtx filesystem-write port — homebrew create path)
    homebrew-create-resume-journaling (journal homebrew create sub-steps / reconcile remote)
    homebrew-formula-non-rust         (generate non-Rust Homebrew formulas)
LANE B — contract schema (crates/ossctl-core/src/contract/schema.rs — SEQUENCE strictly) — POST-RELEASE hardening
    distribution-monorepo-vec            (Vec<Distribution> + per-package association)
    distribution-extra-fields            (extra_fields forward-compat on nested distribution structs)
    distribution-installer-platform-crosscheck (validate installer/platform coherence)
    distribution-platforms-adapter-neutral     (platforms field adapter-neutrality)
LANE C — release CI/pipeline infra (.github/workflows/**, dist-workspace.toml — SEQUENCE strictly; both fix release.yml/adjacent) — reliability, filed stint #11
    publish-crates-no-auto-trigger       (bug — release:published doesn't cascade from GITHUB_TOKEN; crates.io needs manual dispatch)
    homebrew-tap-bump-manual-and-missed  (bug — tap formula bump is manual; silently served stale v0.1.0 through 0.1.1)
    release-macos-hauis-coupling         (improvement — canon release depends on personal hauis runner; decouple to hosted macos-14)
```
<!-- execution-dag:end -->

## Backlog

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN). `issuectl list` for the
live view. 0.1.2 is shipped; the epic stays open for its tails (see handoff) and the lanes above.
