# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-06 (stint #13). New agent: read this, then continue with a fresh
`/stint-start`. Main is clean, all pushed (`aba2798`)._

_🎉 **STINT #13 SHIPPED ossctl 0.2.1** (manual fallback again) — and the engine got ONE LAYER
DEEPER toward self-hosting. One unit this round: the release-engine INTERLEAVE fix
(`release-cut-build-phase-dep-ordering`, commits `ce85309`+`35f9c23`) — landed green (310 tests),
ADR-0002 amended. It WORKS: the 0.2.1 engine cut passed `dry-run-all` **and `build-all`** for all
four targets, including `rust:ossctl:crates.io` — the exact step that killed both 0.2.0 attempts.
That build-phase blocker is DEAD._
_- **crates.io** — `ossctl-core 0.2.1` + `ossctl 0.2.1` (dep order)._
_- **GitHub Release `v0.2.1`** — full cross-platform asset set (macOS aarch64, Linux musl
  x86_64+aarch64, Windows, `.sh`+`.ps1` installers, sha256 sums, source.tar.gz)._
_- **Homebrew tap** — formula bumped `v0.2.0`→`v0.2.1` (tap commit `7d39642`, sha256 verified)._

_⚠️ **0.2.1 WAS CUT MANUALLY, NOT BY THE ENGINE — a NEW, DEEPER blocker.** The 0.2.1 engine cut
(run 01KZBDST…) got PAST build, then failed SAFELY in the PUBLISH phase (pre-upload, nothing shipped,
run abandoned, `published_targets: []`): the registry-aware defer predicate needs a crates.io
**RegistryQuery** to verify a crate's published state, and that is **not wired for ecosystem `rust`
yet**, so the publish path FAILS CLOSED rather than guess. This is now **THE last blocker before the
engine can dogfood its own cut** — `release-publish-registry-query-not-wired` (LANE A, HIGH, NEW this
stint). It **likely overlaps** `cargo-publish-receipt-provenance-resume-safety` (which already calls
for a "RegistryQuery checksum") — NEXT STINT should decide whether to merge them and wire the
crates.io RegistryQuery as ONE unit. Fix it → the 0.2.2 cut is the true engine dogfood._

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

_**Stint #13 also filed (triaged into LANE A below):** the interleave worker's `/llm-review`
produced three cargo-adapter follow-ups — `cargo-metadata-recomputed-per-phase`,
`cargo-build-disposition-journal`, `cargo-interleave-real-cargo-integration-test` — all deferred
hardening, none gate the RegistryQuery fix. (Stint #12's filings remain in the DAG: homebrew
resume-idempotency/stable-tarball, cargo receipt-provenance, target-coverage, journal-identity,
stale-lock-break, resume-publish-phase, release-verify-delegated-release; family/schema gaps
`oss-dist-channel-generator`, `publish-crates-release-trigger`, `publish-target-none`.)_

_--- older history in git: stints #1–7 built the `/oss-*` deterministic core, #8 finished the
adapters, #9 shipped 0.1.0, #10 shipped 0.1.1 cross-platform, #11 shipped 0.1.2 (`dist generate`).
Epic `ossctl-phase4-build` stays OPEN. Cross-repo standardisation + hauis infra remain HOMEBASE
concerns (homebase issue `cross-repo-release-standardisation`), NOT ossctl work. ---_

**Read first (the spec):** `docs/adr/000{1,2,3,4}-*.md` (CLI taxonomy, release engine, config+journal, one-target-one-publish-unit).

## Execution DAG (2026-08-06, stint #13 handoff)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/handoff (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

The cross-repo standardisation ("Track A") and hauis CI runners are HOMEBASE concerns (Jari's
personal environment), NOT ossctl work — moved to homebase issue `cross-repo-release-standardisation`.
Do not re-add them here.

**Track B critical path — "best shape = ossctl cuts ITSELF through the engine".** The endgame is
dogfooding: `ossctl release cut` drives ossctl's own cut end-to-end, retiring the 4-step manual
recipe. STATUS after stint #13: the engine now clears `dry-run-all` + `build-all` for all four
targets (the interleave fix killed the build-phase blocker); it stops at the PUBLISH phase because
the crates.io **RegistryQuery is not wired for `rust`**, so the registry-aware defer/idempotency
decision fails closed. **The whole remaining path is ONE unit:**
`release-publish-registry-query-not-wired` (wire the crates.io RegistryQuery — likely merge with
`cargo-publish-receipt-provenance-resume-safety`). Fix it → 0.2.2 is the dogfood proof. LANE A still
SUBSUMES most of LANE C (homebrew-tap-bump → cargo-dist-flow post-tag phase; publish-crates-* → moot,
engine publishes directly); only `release-macos-hauis-coupling` survives (CI-delegated build,
homebase-adjacent — it 400'd again this stint on the stale hauis token, cleared+rerun). So do NOT
harden LANE C now — it polishes a manual path we intend to retire.

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: release-publish-registry-query-not-wired   (LANE A, HIGH, NEW stint #13 — THE last blocker before the engine can dogfood its own cut. 0.2.1 SHIPPED (2026-08-06) via manual fallback (crates.io ×2 + GH Release + homebrew tap → v0.2.1). The engine now clears dry-run+build for all 4 targets (interleave fix landed); it fails CLOSED at the PUBLISH phase because no crates.io RegistryQuery is wired for `rust`. REAL fix: wire the crates.io RegistryQuery so the publish path can verify published state — likely MERGE with cargo-publish-receipt-provenance-resume-safety. Fix it → 0.2.2 is the true dogfood.)
LANE A — release engine (crates/ossctl-core/src/release/**; SEQUENCE strictly) — Track B: make `ossctl release cut` cut ossctl ITSELF (dogfooding proof)
  [DONE stint #12] release-cut-multi-target-ecosystem       (fixed — >1 target/ecosystem now cut in dep order)
  [DONE stint #12] cargo-adapter-multitarget-double-publish (fixed — Option 1 "one target = one publish unit"; ADR-0004; no more double-publish of ossctl-core)
  [DONE stint #12] release-engine-cut-cargo-dist-flow       (done — coordinator skips CI-delegated targets + post-tag homebrew phase with real sha256)
  [DONE stint #12] coordinator-release-vs-cargo-dist-ownership (done — Option 1: coordinator delegates GH Release to CI when a CI-delegated target is present; ci_owns_github_release flag, seal v3)
  [DONE stint #12] cargo-publish-pin-crates-io-registry     (fixed — pinned cargo adapter to crates.io, rejects other registries)
  [DONE stint #12] release-list-abandon-not-implemented     (fixed — `release list` + `abandon` implemented over the journal; in-flight gate + recovery net)
  [DONE stint #13] release-cut-build-phase-dep-ordering     (fixed — INTERLEAVE build+publish for =-pinned cargo dependents: adapter defers a dependent's packaging into its dep-ordered `cargo publish` (registry-aware, fail-closed); outer barrier + coordinator-only tag + post-tag homebrew preserved; ADR-0002 amended; interleave+resume tests. The 0.2.1 engine cut proved it PASSES build-all.)
  --- engine now clears dry-run+build for all 4 targets; STOPS at publish (RegistryQuery gap) ---
  ▶ release-publish-registry-query-not-wired (bug, HIGH, NEW stint #13 — publish phase fails CLOSED on the first crates.io target: "no registry query wired for ecosystem 'rust' yet" — the registry-aware defer/idempotency predicate can't verify a crate@version's published state. SAFE: fails before any upload (0.2.1 cut abandoned, published_targets:[]). Fix: wire a crates.io RegistryQuery (index/API: exists? checksum?). LIKELY MERGE with cargo-publish-receipt-provenance-resume-safety below (which already needs a "RegistryQuery checksum"). Preserve fail-closed on genuine unreachable.)
    << then CUT 0.2.2 THROUGH the engine >> — the dogfood proof that retires the 4-step manual recipe.
     · PROCEDURE (contract already declares all 4 targets + distribution block):
     · (1) bump 0.2.1→0.2.2 + internal =-pin in lockstep, finalize CHANGELOG, `cargo build`, commit+push;
     · (2) `ossctl release plan --version 0.2.2` (seal + inspect, side-effect-free);
     · (3) `ossctl release cut --plan <id> --version 0.2.2` (autonomous per policy);
     · (4) post-cut `gh release view v0.2.2` (Option 1 delegates the Release to CI; hauis may 400 →
     ·     clear token + `gh run rerun <id> --failed`), until release-verify-delegated-github-release
     ·     automates it. crates.io is irreversible — plan/dry-run first, never cut red. >>
  --- stint #13 review follow-ups (cargo adapter; deferred, do not gate the RegistryQuery fix) ---
    cargo-metadata-recomputed-per-phase  (improvement — cargo metadata recomputed per phase; cache it. Filed by interleave /llm-review)
    cargo-build-disposition-journal      (improvement — journal the per-target build disposition (defer vs package). Filed by interleave /llm-review)
    cargo-interleave-real-cargo-integration-test (task — add a real-cargo integration test exercising the interleave end-to-end. Filed by interleave /llm-review)
  --- production-safe hardening (deferred PAST the dogfood cut — do not gate it on these) ---
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
