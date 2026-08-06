# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-06 (stint #12). New agent: read this, then continue with a fresh
`/stint-start`. Main is clean, all pushed (`d61096e`)._

_🎉 **STINT #12 SHIPPED ossctl 0.2.0** — the release engine can now (in code) drive a
multi-target, cross-channel cut. Landed & green (7 LANE A units): multi-target-per-ecosystem
in dep order; **one-target-one-publish-unit (ADR-0004)** — no double-publish; cargo-dist flow
(skip CI-delegated targets + post-tag homebrew with real sha256); **Release-ownership Option 1**
(coordinator delegates the GitHub Release to CI when a CI-delegated target is present,
`ci_owns_github_release`); crates.io registry pinning; and `release list`/`abandon`. Contract
now declares a `distribution` block with `homebrew_tap: jarimustonen/homebrew-ossctl`._
_- **crates.io** — `ossctl-core 0.2.0` + `ossctl 0.2.0` (dep order)._
_- **GitHub Release `v0.2.0`** — full cross-platform asset set (macOS aarch64, Linux musl
  x86_64+aarch64, Windows, `.sh`+`.ps1` installers, sha256 sums, source.tar.gz)._
_- **Homebrew tap** — formula bumped `v0.1.2`→`v0.2.0` (tap commit `7eb3034`, sha256 verified)._

_⚠️ **0.2.0 WAS CUT MANUALLY, NOT BY THE ENGINE.** Two engine cuts (`ossctl release cut`) failed
SAFELY at the build phase (pre-publish, nothing shipped, runs abandoned): `cargo package -p ossctl`
resolves the `=`-pinned `ossctl-core` dep against the crates.io INDEX even with `--no-verify`, so a
dependent can't be packaged until its dep is actually PUBLISHED. The `build-all → publish-all`
barrier is incompatible with cargo's multi-crate `=`-pinned model. **This is THE one remaining
blocker before the engine can dogfood its own cut** — `release-cut-build-phase-dep-ordering`
(LANE A, HIGH, REOPENED). Real fix: **interleave build+publish per dep-ordered cargo target**
(publish core → wait index → package+publish cli) — an ADR-0002 amendment; **consider
`/worktree-technical-decision`** since it changes a core invariant. Fix it → **the 0.2.1 cut is
the true engine dogfood.** The 0.2.0 prep artifacts (version bump, finalized CHANGELOG, contract
distribution block) are already on `main`._

_**Manual fallback recipe (used for 0.2.0; use until the engine cut works):** bump version + internal
`=X.Y.Z` dep in lockstep → finalize CHANGELOG → `cargo build` → `cargo publish -p ossctl-core`
(real) → cargo waits for the index → `cargo publish -p ossctl` → `git tag vX.Y.Z && git push origin
vX.Y.Z` → the tag triggers cargo-dist `release.yml` (macOS aarch64 on **hauis**; if it 400s with
`Duplicate header: Authorization`, `ssh hauis 'git config --global --unset-all
"http.https://github.com/.extraheader"'` then `gh run rerun <run-id> --failed`) → after the GH
Release exists, bump the Homebrew tap by hand (`curl -sL <tag-archive>; shasum -a 256`; PUT
`Formula/ossctl.rb` in `jarimustonen/homebrew-ossctl`). All four hauis/publish-crates/tap defects
still tracked in LANE C, but MOSTLY SUBSUMED once the engine cut lands._

_**Operating policy (see AGENTS.md — updated this stint):** (1) releases may be cut AUTONOMOUSLY;
(2) **the engine-driven `ossctl release cut` is fully autonomous — NO go/no-go checkpoint, ever**
(incl. first cut + homebrew leg); safety is structural (`release plan` seal + `dry-run-all` +
dep-order/index-wait + `resume`/`abandon`), never a human gate; (3) `git pull --rebase` → `push`
always allowed. Green gate incl. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`._

_**Stint #12 also filed (all triaged into the DAG below):** the review cascade produced deferred
hardening (homebrew resume-idempotency/stable-tarball, cargo receipt-provenance, target-coverage,
journal-identity, stale-lock-break, resume-publish-phase, release-verify-delegated-release) plus
worker-filed family/schema gaps (`oss-dist-channel-generator`, `publish-crates-release-trigger`,
`publish-target-none`). None gate the interleave fix — that is the single next thing._

_--- older history in git: stints #1–7 built the `/oss-*` deterministic core, #8 finished the
adapters, #9 shipped 0.1.0, #10 shipped 0.1.1 cross-platform, #11 shipped 0.1.2 (`dist generate`).
Epic `ossctl-phase4-build` stays OPEN. Cross-repo standardisation + hauis infra remain HOMEBASE
concerns (homebase issue `cross-repo-release-standardisation`), NOT ossctl work. ---_

**Read first (the spec):** `docs/adr/000{1,2,3,4}-*.md` (CLI taxonomy, release engine, config+journal, one-target-one-publish-unit).

## Execution DAG (2026-08-06, stint #12 handoff)

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
    release-abandon-reason-leading-dashes (bug, minor CLI — `release abandon --reason` rejects a reason starting with `--` (clap parses it as a flag). Filed stint #12 wrap)
    plan-preimage-projection          (release/plan: hash a release-relevant projection, not the whole Contract)
    seal-verify-drift-dx              (release/plan: ergonomic SEAL_VERSION bump + golden-vector regen)
    homebrew-adapter-fs-port          (EffectCtx filesystem-write port — homebrew create path)
    homebrew-create-resume-journaling (journal homebrew create sub-steps / reconcile remote)
    homebrew-formula-non-rust         (generate non-Rust Homebrew formulas)
LANE B — contract schema (crates/ossctl-core/src/contract/schema.rs + normalize.rs — SEQUENCE strictly) — POST-RELEASE hardening
    publish-target-none                  (feature — contract can't express "version-tracked + changelogged but NEVER published"; normalizer force-expands a registry target, registry enum has no non-publishing value. Surfaced on another project. Filed via 63c7c7c)
    contract-homebrew-tap-warning-false-positive (bug — normalize.rs "homebrew_tap set but no homebrew installer" warning ignores a homebrew-tap TARGET as a consumer; false-positive on ossctl's own (correct) contract. Cosmetic. Filed stint #12 wrap)
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
