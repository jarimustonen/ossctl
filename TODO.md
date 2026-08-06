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

## Execution DAG (2026-08-05, stint #12)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/handoff (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

The cross-repo standardisation ("Track A") and hauis CI runners are HOMEBASE concerns (Jari's
personal environment), NOT ossctl work — moved to homebase issue `cross-repo-release-standardisation`.
Do not re-add them here.

**Track B critical path (decided stint #12) — "best shape = ossctl cuts ITSELF through the engine".**
The endgame that puts ossctl in best shape is dogfooding: `ossctl release cut` drives ossctl's own
0.2.0 end-to-end, retiring the 4-step manual recipe. Shortest path is a STRICT LANE A sequence (all
same release-engine hot files — NOT parallelisable): (1) `release-cut-multi-target-ecosystem` [the
lock — 2 crates.io targets], (2) `release-engine-cut-cargo-dist-flow` [cut logic: local publish +
delegate cross-platform build to tag-triggered CI + post-tag homebrew], (3) `release-list-abandon`
[recovery safety], then (4) cut 0.2.0 through the engine as the proof. LANE A SUBSUMES most of LANE C:
homebrew-tap-bump → the cargo-dist-flow post-tag phase; publish-crates-no-auto-trigger → moot (engine
publishes crates directly). Only `release-macos-hauis-coupling` survives (CI-delegated build) and it's
homebase-adjacent. So do NOT harden LANE C now — that polishes a manual path we intend to retire.

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: release-cut-build-phase-dep-ordering   (LANE A, REOPENED — the LAST blocker before the engine can dogfood its own cut. 0.2.0 SHIPPED (2026-08-06) via the manual fallback (crates.io ×2 + GH Release + homebrew tap bumped to v0.2.0). Two engine cuts failed SAFELY here; --no-verify insufficient. REAL fix: interleave build+publish per dep-ordered cargo target (ADR-0002 amendment; consider /worktree-technical-decision). Fix it → then the 0.2.1 cut is the true engine dogfood)
LANE A — release engine (crates/ossctl-core/src/release/**; SEQUENCE strictly) — Track B: make `ossctl release cut` cut ossctl ITSELF (0.2.0 dogfooding proof)
  [DONE stint #12] release-cut-multi-target-ecosystem       (fixed — >1 target/ecosystem now cut in dep order)
  [DONE stint #12] cargo-adapter-multitarget-double-publish (fixed — Option 1 "one target = one publish unit"; ADR-0004; no more double-publish of ossctl-core)
  [DONE stint #12] release-engine-cut-cargo-dist-flow       (done — coordinator skips CI-delegated targets + post-tag homebrew phase with real sha256)
  [DONE stint #12] coordinator-release-vs-cargo-dist-ownership (done — Option 1: coordinator delegates GH Release to CI when a CI-delegated target is present; ci_owns_github_release flag, seal v3)
  [DONE stint #12] cargo-publish-pin-crates-io-registry     (fixed — pinned cargo adapter to crates.io, rejects other registries)
  [DONE stint #12] release-list-abandon-not-implemented     (fixed — `release list` + `abandon` implemented over the journal; in-flight gate + recovery net)
  --- TWO 0.2.0 engine cuts ATTEMPTED, both failed SAFELY (pre-publish); engine needs an ARCHITECTURAL fix ---
  ▶ release-cut-build-phase-dep-ordering (bug, HIGH, REOPENED — `cargo package -p ossctl` resolves `ossctl-core="=0.2.0"` vs the crates.io INDEX even with `--no-verify`, so the dependent can't be packaged until the dep is actually PUBLISHED. Strict build-all→publish-all barrier is incompatible with cargo's multi-crate `=`-pinned model. REAL fix: interleave build+publish per dep-ordered cargo target (publish core → wait index → package+publish cli) — ADR-0002 amendment; consider /worktree-technical-decision. Kept: the dry-run-mirror change (fails at dry-run now, safer). See issue note.)
    << then RE-CUT 0.2.0 THROUGH the engine (0.2.0 prep bump+changelog+contract already committed & pushed on main) >> — the dogfooding proof that retires the 4-step manual recipe.
     · PROCEDURE (1) update ossctl's OSS-RELEASE.md contract to declare the full target set
     · — 2 crates.io (ossctl-core + ossctl) + gh-releases/cargo-dist + homebrew — if not already;
     · (2) `ossctl release cut --dry-run` / seal-plan stage to validate the plan WITHOUT publishing;
     · (3) real engine cut; (4) manual post-cut `gh release view v0.2.0` check that CI created the
     · Release + assets (Option 1 delegates it), until release-verify-delegated-github-release
     · automates it. crates.io is irreversible — dry-run first, never cut red. >>
  --- production-safe hardening (deferred PAST the first cut — do not gate 0.2.0 on these) ---
    resume-publish-phase-never-reached  (bug — `release resume` demands --allow-unverified even when publish phase was never entered; safe to pass it meanwhile. Filed by list-abandon review)
    release-abandon-break-stale-lock    (improvement — `abandon` can't auto-break a stale single-active-cut lock after a hard-kill; shipped stopgap names the lock-file path for manual clearing. All 4 reviewers flagged. Filed by list-abandon review)
    journal-open-identity-structural-hardening (improvement — validate journal identity/structure on read; needs a corrupt journal, low real-world likelihood. Filed by list-abandon review)
    release-verify-delegated-github-release (task — `ossctl release verify` should query GitHub to confirm CI actually created a delegated Release, instead of assuming success. Automates the manual post-cut check above. Filed by ownership review)
    homebrew-publish-resume-idempotency (bug — homebrew adapter not idempotent on resume → duplicate PR if a cut dies mid-homebrew (no natural dup-guard like crates.io). All 3 reviewers flagged. HIGHER-STAKES defer: only bites an interrupted cut. Filed by cargo-dist-flow review)
    homebrew-stable-source-tarball       (improvement — GH auto-archive not byte-stable; build+upload a deterministic source tarball long-term. Issue says NOT a blocker. Filed by cargo-dist-flow review)
    cargo-publish-receipt-provenance-resume-safety (bug cluster — receipts carry no content digest → resume/reconcile can't prove provenance; needs RegistryQuery checksum + attempt-journaling + new AdapterError variants. Large; "production-safe end-to-end". Filed by ADR-0004 review)
    cargo-target-coverage-preflight      (feature — plan-time reject of under-declared plans (fail-fast vs 300s publish-time timeout). ossctl's own contract is correctly declared so it won't trigger for us. Filed by ADR-0004 review)
    cargo-per-member-receipts        (per-member publish receipts for multi-crate cuts — likely folds into receipt-provenance above)
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
LANE C — release CI/pipeline infra (.github/workflows/**, dist-workspace.toml — SEQUENCE strictly) — MOSTLY SUBSUMED BY LANE A (see note), keep only as insurance for the manual/fallback recipe + other repos
    publish-crates-no-auto-trigger       (bug — moot for ossctl's engine cut (engine publishes crates directly); DUP-cluster with publish-crates-release-trigger below)
    publish-crates-release-trigger       (bug — same root as above with a VERIFIED fix: trigger publish-crates.yml on the version-tag push, not release:published (GITHUB_TOKEN emits no release event). Proven in glasspad. Fold with publish-crates-no-auto-trigger. Filed via 0f7c637)
    homebrew-tap-bump-manual-and-missed  (bug — SUBSUMED by LANE A cargo-dist-flow's post-tag homebrew phase)
    release-macos-hauis-coupling         (improvement — the ONE LANE C survivor: cross-platform build is CI-delegated so engine can't own it. Personal hauis infra → arguably a HOMEBASE concern; do last / defer)
UNLANED — /oss-* family completeness (skill/template work; no release-engine hot file; run anytime):
    oss-dist-channel-generator           (feature — no /oss-* member generates the distribution channel (dist-workspace.toml + release.yml + tap scaffold + secrets) from a contract's gh-releases/homebrew targets; `ossctl dist generate` does the config half but no skill wraps it + scaffolds the tap. Surfaced on glasspad 0.2.1. Filed via 0f7c637)
```
<!-- execution-dag:end -->

## Backlog

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN). `issuectl list` for the
live view. 0.1.2 is shipped; the epic stays open for its tails (see handoff) and the lanes above.
