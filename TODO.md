# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-05 (stint #10). New agent: read this, then continue with a fresh
`/stint-start`. Main is clean, all pushed._

_🎉 **STINT #10 SHIPPED ossctl 0.1.1 — the first CROSS-PLATFORM release (macOS + Linux).**
Fully published on all channels:_
_- **crates.io** — `ossctl-core 0.1.1` + `ossctl 0.1.1` (published in dep order via CI)._
_- **GitHub Release `v0.1.1`** — 14 assets: Linux musl binaries (`x86_64` + `aarch64`),
  macOS `aarch64`, Windows, a `curl … | sh` shell installer (`ossctl-installer.sh`) + `.ps1`,
  source, sha256 sums._
_- Installs on Linux three ways: `cargo install ossctl`, the shell installer, or the prebuilt
  musl binary. `brew install jarimustonen/ossctl/ossctl` still works._

_HOW 0.1.1 was cut (READ — this is the current release process): **NOT `ossctl release cut`**
(the engine still can't safely drive the cargo-dist + homebrew flow — see "engine gap" below).
It was cut by the **CI recipe**, now standardised: bump `workspace.package.version` + the
internal `=X.Y.Z` dep in lockstep → finalize CHANGELOG → commit → `git push --tags` a `vX.Y.Z`
tag → the tag triggers `.github/workflows/release.yml` (cargo-dist: builds the cross-platform
binary matrix + shell installer + creates the GitHub Release) → `release: published` fires
`.github/workflows/publish-crates.yml` (publishes `ossctl-core` then `ossctl` to crates.io in
dep-order). **crates.io publish is CI-side**, fed by a `CARGO_REGISTRY_TOKEN` repo secret — no
local `cargo login` needed. (During this stint a local `cargo publish` 403'd on a stale token;
that's why we moved publishing into CI.)_

_**Operating-policy changes landed this stint (in AGENTS.md, both apply going forward):**_
_1. **Releases may be cut AUTONOMOUSLY** whenever main has something to release — no per-release
   go. Green gate must pass; `cargo publish` dry-runs first; crates.io is irreversible so never
   publish red._
_2. **`git pull --rebase` → `push` is always allowed autonomously** on this repo (overrides the
   global "pushing is the user's step" default). Never force-push a shared branch; never push red._
_Also: the green gate now includes `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
(CI's `docs` job caught broken intra-doc links that fmt/clippy/test missed — run it on any unit
touching doc comments)._

_**⭐ THE MAIN NEXT OBJECTIVE — multi-repo standardisation (user directive).** Make all FOUR
Rust CLI repos — ossctl, issuectl, orchestratectl, glasspad — build + release the SAME way
(through ossctl's pattern) with IDENTICAL operating policies. Agreed sequence with the user:
**A → 0.1.1 (done) → B → 0.2.0.** ossctl is fully standardised; the rollout to the other three
is the checkpoint's remaining work._

_**Track A rollout — remaining (do these next, one repo at a time; each is a separate git repo
under `~/Sources/`):**_
_- **issuectl** — has `release.yml` + `publish-crates.yml` + hauis runner. NEEDS: wire
  `[dist.github-custom-runners]` (aarch64-apple-darwin → `self-hosted`) into its
  `dist-workspace.toml` (mac = aarch64-only, see hauis gotcha) + regenerate `release.yml` via
  `dist generate`; apply the identical operating-policy block._
_- **orchestratectl** — has `release.yml` + hauis runner + already routes mac to hauis
  (aarch64-only). NEEDS: add `publish-crates.yml` (mirror ossctl's 2-crate/1-crate form); apply
  the identical operating-policy block._
_- **glasspad** — single-crate Rust CLI, has ONLY `OSS-RELEASE.md` (no release infra). NEEDS the
  FULL setup from scratch: cargo-dist `dist-workspace.toml` (cross-platform: Linux musl x86_64+arm64,
  macOS aarch64 → hauis, Windows) → `dist generate` `release.yml`; `publish-crates.yml`; the
  `CARGO_REGISTRY_TOKEN` secret is already set; apply the identical operating-policy block._
_- **ALL THREE**: copy the two operating-policy grants + the cross-platform requirement VERBATIM
  from ossctl's `AGENTS.md` "Operating policy" section (they are the canon)._

_**Track B — then, in ossctl, for a genuinely engine-driven 0.2.0:**_
_- `release-engine-dist-config-generator` (feature) — ossctl GENERATES the standard release infra
  (dist-workspace.toml + release.yml + publish-crates.yml) for any repo, so it's generated not
  hand-copied. This is what makes "through ossctl" real._
_- The engine-cut gap (file it if not present): `ossctl release cut` still can't drive ossctl's
  own flow — the `gh-releases/cargo-dist` target returns `Unsupported` in `publish()` (a
  partial-publish trap: crates.io would publish then the cut sticks, no rollback), and the
  homebrew tarball sha256 only exists post-tag (engine opens a draft PR needing a hand-filled
  hash). Landing a coordinator that SKIPS CI-delegated targets + does post-tag homebrew makes
  0.2.0 the first ENGINE-driven cut._

_**hauis CI infrastructure (NEW this stint — documented in homebase
`infra/machines/hauis.md`).** hauis is a separate Mac (`ssh hauis`), NOT gertrud (this dev box).
All four repos now have a self-hosted GitHub Actions runner named `hauis`
(`self-hosted,macOS,ARM64`) so their macOS release builds run on hauis (~2 min) instead of
GitHub's hosted macOS queue (was 45+ min just to allocate). GOTCHAS learned the hard way:_
_- **One runner instance PER repo** (User-account runners are repo-scoped): `~/actions-runner/`
  = orchestratectl (original), `~/actions-runner-{ossctl,issuectl,glasspad}/` = the new three,
  each a launchd service `actions.runner.jarimustonen-<repo>.hauis`._
_- **Fresh-download, don't copy** an existing runner dir — a recent runner stores config in
  `.runner_migrated` (not `.runner`), so a copy trips "already configured". Full recipe is in
  the homebase doc._
_- **hauis uses Homebrew Rust, NO rustup** → it CANNOT cross-compile `x86_64-apple-darwin`.
  So the mac target is **aarch64-only** on every repo (Intel-mac users use `cargo install`).
  Same choice orchestratectl already made._
_- **`dist-workspace.toml` `[dist.github-custom-runners]` MUST be at the END** of the `[dist]`
  table (a `[dist.*]` sub-table among the scalar keys swallows every key after it — this broke
  `github-attestations`). It's a PERSONAL/NON-STANDARD override, clearly marked; ossctl never
  emits it by default (other users have no hauis)._
_- If a runner's checkout fails with **HTTP 400**, a stale `http.https://github.com/.extraheader`
  is stuck in hauis's GLOBAL git config (leftover token from a killed job):
  `ssh hauis 'git config --global --unset-all "http.https://github.com/.extraheader"'`._

_**Two new engine bugs filed by a parallel session** (dogfooding ossctl on orchestratectl's cut):
`release-cut-multi-target-ecosystem` + `release-list-abandon-not-implemented` (both in LANE A)._

_**Stint #10 issue work (all landed, closed `fixed`, all reviewed; 342 tests green at cut):**
the two `/oss-init` dogfood-feedback issues (`maturity-inference-pre-1-0-production`,
`contract-cannot-model-cargo-dist-release`), the release-engine completion
(`cargo-adapter-workspace-publish`, `homebrew-adapter-first-formula`, `gh-release-ci-workflow`,
`adapter-publish-completeness` → all 6 adapters REAL/honest), and the cross-platform campaign
(`distribution-cross-platform-targets` = `distribution.platforms` Linux-by-default,
`oss-readme-cross-platform-install`, `oss-release-cross-platform-dist`, `audit-cross-platform-gap`,
`ossctl-readme-refresh` + AGENTS cross-platform policy, `linux-release-binaries`). Their reviews
spun off the post-release hardening backlog in the DAG below._

_ALL remaining open issues are POST-RELEASE hardening/future work — none block a release._

_--- older history (stints #1–9) is in git; the short version: stints #1–7 built the whole
`/oss-*` deterministic core (contract/facts/audit/skill/release-engine + 9 bundled skills),
#8 finished the adapters, #9 shipped 0.1.0 by hand. Epic `ossctl-phase4-build` stays OPEN
(tails: `migrate-oss-init` deferred until the homebase `/oss-init` copy is removed —
`dotfiles/src/.claude/skills/oss-init/`; and the non-Rust adapter build-side skeletons). ---_

**Read first (the spec):** `docs/adr/000{1,2,3}-*.md` (CLI taxonomy, release engine, config+journal).

## Execution DAG (2026-08-05)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/7 (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

Stint #10 shipped 0.1.1 cross-platform via the CI recipe. The lanes below are ossctl's own
POST-RELEASE backlog; `release-engine-dist-config-generator` (Track B) is the priority path
toward an engine-driven 0.2.0 and is ossctl's actual next objective.

NOTE (scope correction, stint #11): the earlier "multi-repo Track A rollout" +
hauis-runner infra are Jari's PERSONAL ENVIRONMENT concerns — they belong in the
**homebase** repo, NOT here. ossctl the product does not own the cross-repo standardisation
of issuectl/orchestratectl/glasspad, nor the self-hosted CI runners. Do not treat those as
ossctl priorities. (The generic capability that MIGHT serve such a rollout —
`release-engine-dist-config-generator` — is a legitimate ossctl feature and is LANE A head;
its downstream USE across Jari's repos is a homebase matter.)

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: release-engine-dist-config-generator   (Track B toward engine-driven 0.2.0 — the ossctl-native priority)
LANE A — release engine (crates/ossctl-core/src/release/**; SEQUENCE strictly) — POST-RELEASE
  ▶ release-engine-dist-config-generator (BUILD the downstream cargo-dist config generator — makes "through ossctl" real; Track B)
    release-engine-cut-cargo-dist-flow   (skip CI-delegated targets + post-tag homebrew — engine-driven 0.2.0 enabler; Track B)
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
```
<!-- execution-dag:end -->

Note: the cross-repo standardisation ("Track A") and hauis CI runners are HOMEBASE concerns
(Jari's personal environment), not ossctl work — see the scope-correction note above. ossctl's
own priority is Track B (`release-engine-dist-config-generator`).

## Backlog

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN). `issuectl list` for the
live view. The 0.1.1 release is shipped; the epic stays open for its tails (see handoff) and the
post-release lanes above.
