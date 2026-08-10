# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-10 (stint #16). New agent: read this, then continue with a fresh
`/stint-start`. Main is clean, all pushed. Live: **0.2.5** on all four channels._

_🎉 **STINT #16 — 0.2.5 SHIPPED. The engine's real-cut publish is now TRUSTWORTHY.** This stint fixed
the two HIGH bugs that the first real downstream cut (issuectl 0.8.1) exposed, then — on Jari's call —
built the REAL no-op fix + single-source version and cut 0.2.5 carrying all of it. Fully-autonomous
engine self-cut, exit 0, all four channels verified live._

_**0.2.5 (2026-08-10)** — `ossctl release cut` self-cut: dry-run-all → build-all → publish-all
(ossctl-core + ossctl → crates.io) → tag v0.2.5 → dist (homebrew direct tap-write). GH Release + 14
cross-platform assets delegated to cargo-dist CI (**succeeded, hauis macOS aarch64 clean, no 400**).
All four channels verified live at 0.2.5 (crates.io ×2 via sparse index; GitHub Release v0.2.5;
Homebrew tap → v0.2.5). Plan `a59584a3…`, head `ccbab61`._

_**What 0.2.5 contains (all user-facing):**_
_- `cut-noop-self-visibility-check` (HIGH, the real fix) — after the irreversible `cargo publish` the
  cargo adapter now CONFIRMS the target's own `{name,version}` reached the crates.io index (reusing the
  bounded index-wait, so normal propagation lag is tolerated) BEFORE journaling a receipt. A silent
  no-op upload now fails the cut CLOSED with no fabricated receipt; a registry outage fails closed
  distinctly. **Proven live on 0.2.5's own self-cut** (passed on a real upload without flakiness)._
_- `release-version-single-source` — the release version is derived from the workspace manifest (single
  source of truth); `--version` is now an OPTIONAL must-match confirmation that subsumes the drift guard._
_- `release-cut-publish-noop` (HIGH) — the `--version`-vs-tree drift guard + mock-registry integration
  test + root-cause analysis (first-round landing; the definitive fix is the self-visibility check above)._
_- `publish-crates-yml` (HIGH regression) — both `cargo publish` lines in the dep-order CI step tolerate
  cargo's exact "already exists on crates.io index" diagnostic as success (anchored match) → no more
  spurious red release runs when the tag-push publish races the engine's own publish. CI-only._

_**Why the critical bug's root cause was NOT a reproduced source no-op:** the worker judged the issuectl
timeout most consistent with an env/registry-token difference OR issuectl-core not declared as its own
release target — not reproducible locally without crates.io creds. The self-visibility check is the
structural defense: whatever the cause, a cut that doesn't actually upload now fails loudly instead of
faking success. If a real downstream cut still no-ops after this, capture the exact emitted `cargo
publish` line, the target manifest versions, and whether every publishable crate is a declared target._

_⚠️ **NO HIGH blocker remains.** Everything left is DEFERRED/optional. Newly filed but NOT yet acted:
none this stint. The noop `/llm-review` surfaced 4 MED/LOW follow-up ideas (NOT filed — file on demand):
fail-closed version guard for manifest-versioned node/python, revisit the `is_published` idempotency
short-circuit, drop `--version` entirely, run cut/resume from a clean checkout of the sealed HEAD._

_**Housekeeping:** no lingering worktrees (all three round workers settled + torn down). A Dependabot
`clap-4.6.5` PR is open on the remote — adjacent, not triaged this stint._

_**hauis note:** 0.2.5's CI macOS aarch64 build on `hauis` succeeded with NO 400 — token healthy. If a
future cut 400s: `ssh hauis 'git config --global --unset-all "http.https://github.com/.extraheader"'`
then `gh run rerun <run-id> --failed`. Tracked as `release-macos-hauis-coupling` (homebase-adjacent)._

_**Operating policy (see AGENTS.md):** (1) releases may be cut AUTONOMOUSLY; (2) the engine-driven
`ossctl release cut` is fully autonomous — NO go/no-go, ever (proven again with 0.2.5); safety is
structural (`release plan` seal + `dry-run-all` + dep-order/index-wait + **the new post-publish
self-visibility confirm** + `resume`/`abandon`); (3) `git pull --rebase` → `push` always allowed. Green
gate incl. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`._

_--- older history in git: stints #1–7 built the `/oss-*` deterministic core, #8 finished the
adapters, #9–11 shipped 0.1.0/0.1.1/0.1.2, #12 multi-target cut, #13 interleave + 0.2.1, #14 completed
the DOGFOOD (0.2.2/0.2.3 via engine) — `ossctl release cut` cuts ossctl itself end-to-end. #15 shipped
0.2.4 + cleared all decisions. #16 shipped 0.2.5 — made the real-cut publish trustworthy (self-visibility
confirm + single-source version). Epic `ossctl-phase4-build` stays OPEN. Cross-repo standardisation +
hauis infra remain HOMEBASE concerns (homebase issue `cross-repo-release-standardisation`), NOT ossctl
work. ---_

**Read first (the spec):** `docs/adr/000{1,2,3,4}-*.md` (CLI taxonomy, release engine, config+journal, one-target-one-publish-unit).

## Execution DAG (2026-08-10, stint #16 handoff)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/handoff (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

The cross-repo standardisation ("Track A") and hauis CI runners are HOMEBASE concerns (Jari's
personal environment), NOT ossctl work — moved to homebase issue `cross-repo-release-standardisation`.
Do not re-add them here.

**Track B — "ossctl cuts ITSELF through the engine" — ✅ COMPLETE (stint #14) and now ROUTINE (stints
#15/#16 shipped 0.2.4/0.2.5 the same way; #16 also made the real-cut publish trustworthy).** No HIGH
blocker remains. Every node below is DEFERRED hardening,
review follow-ups, or the one approved future feature (`oss-dist-channel-generator`, UNLANED). LANE C
is retired for ossctl's own cut — only `release-macos-hauis-coupling` survives (homebase-adjacent).
**Do NOT harden LANE C.** Pick any deferred node for autonomous progress; nothing gates anything.

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: stint #16 round-2 — Jari APPROVED doing ALL 4 cut-noop review follow-ups (2026-08-10). NO HIGH blocker remains; these are MED hardening. All 4 are LANE A engine-core → SEQUENCE strictly, run as 3 sequenced workers:
  Worker A ▶ release-drop-version-flag + version-source-fail-closed-nonrust (same version-resolution path). ⚠️ release-drop-version-flag is a BREAKING CLI change → its release is 0.3.0, not 0.2.6.
  Worker B → release-cut-clean-checkout (after A).
  Worker C → is-published-digest-authenticate (after B; may fold into the receipt-provenance cluster).
  Then CUT 0.3.0 (breaking: --version removed). After: per-distribution-release, release-verify-delegated-github-release, release-cut-stale-binary-guard, the rest of the deferred list. FEATURE (approved, own stint): oss-dist-channel-generator via /worktree-make-skill.
LANE A — release engine (crates/ossctl-core/src/release/**; SEQUENCE strictly)
  [DONE stint #15] resume-publish-phase-never-reached      (fixed — resume no longer demands --allow-unverified when the publish phase was never reached; unsafe rows unchanged)
  [DONE stint #15] release-abandon-reason-leading-dashes   (fixed — `release abandon --reason` accepts values starting with `--` via allow_hyphen_values)
  [DONE stint #15] homebrew-tapwrite-preserve-formula      (done — ownership marker: full-regen only when marked, else surgical url/sha edit or fail-closed refusal; hand-maintained formulas preserved)
  [DONE stint #16] release-cut-publish-noop (fixed — landed a --version-vs-tree-manifest drift guard on plan/cut/resume + a mock-registry cut integration test asserting versions actually land with a per-member receipt + root-cause analysis. NOT a reproduced source no-op: worker judged the issuectl timeout most consistent with an env/registry-token difference OR issuectl-core not declared as its own target — not reproducible locally w/o crates.io creds. REAL fix (post-publish self-visibility check) deferred to cut-noop-self-visibility-check as a maintainer decision (behavior change on the irreversible phase))
  [DONE stint #16] cut-noop-self-visibility-check (fixed — after cargo publish the adapter confirms the target's OWN {name,version} reached the crates.io index (bounded index-wait) before journaling a receipt; a silent no-op fails the cut closed, an outage fails closed distinctly. Proven live on the 0.2.5 self-cut. 4-model /llm-review applied)
  [DONE stint #16] release-version-single-source (done — release version derived from the workspace manifest (single source of truth); --version is now an optional must-match confirmation that subsumes the stint #16 drift guard. plan.rs/release.rs)
  --- ACTIVE stint #16 round-2 (Jari approved all 4 cut-noop review follow-ups; SEQUENCE strictly, 3 workers) ---
  ▶ release-drop-version-flag            (improvement — Worker A part 1. Drop --version as a version INPUT; manifest is the ONLY source. BREAKING CLI change: update the AGENTS.md recipe + /oss-* skill templates + tests. Finishes release-version-single-source. → its release is 0.3.0)
    version-source-fail-closed-nonrust   (improvement — Worker A part 2. Version-drift + self-visibility guards fail OPEN for manifest-versioned non-Rust (node/python); add an ecosystem version-source capability model that fails CLOSED for a manifest-versioned ecosystem whose version can't be read, vs legitimately skips homebrew/binary/cargo-dist. Same version path as release-drop-version-flag → same worker)
    release-cut-clean-checkout           (improvement — Worker B, after A. Run cut/resume from a clean checkout of plan.head_sha instead of the live mutable tree; closes the mid-cut TOCTOU. LOW urgency)
    is-published-digest-authenticate     (improvement — Worker C, after B. Digest-authenticate the is_published resume skip (checksum-match the on-registry crate before trusting the skip); the last receipt-without-fresh-upload path. May fold into cargo-publish-receipt-provenance-resume-safety)
  --- DEFERRED hardening + review follow-ups (none blocks anything) ---
    release-cut-stale-binary-guard       (improvement — filed stint #16 wrap. plan/cut run stale ENGINE code silently when the binary wasn't built from the current tree (the drift guard checks --version vs manifest, NOT binary provenance; since 0.2.5 the tree-read version masks a stale binary). Warn/error when compiled commit != HEAD. Hit by hand during the 0.2.5 cut. NOT urgent — only bites a mis-built binary)
    per-distribution-release             (feature — NEW: per-distribution taps in the engine, `dist generate --package`, cargo-dist per-distribution platforms. The monorepo contract MODEL is complete (distribution-monorepo-vec, 0.2.4); the engine is single-distribution and fails loud on len>1. Filed by distribution-monorepo-vec /llm-review)
    release-verify-delegated-github-release (task — `ossctl release verify` should query GitHub to confirm CI actually created the delegated Release, instead of assuming success. Automates the manual post-cut `gh release view` check. Filed by ownership review)
    homebrew-publish-resume-idempotency  (bug — homebrew adapter not idempotent on resume → duplicate PR/push if a cut dies mid-homebrew (no natural dup-guard like crates.io). Non-ff push race + stale-version downgrade also here. HIGHER-STAKES defer: only bites an interrupted cut)
    cargo-publish-receipt-provenance-resume-safety (bug cluster — receipts carry no content digest → resume/reconcile can't prove provenance; needs RegistryQuery checksum + attempt-journaling + new AdapterError variants. Large; "production-safe end-to-end")
    cargo-per-member-receipts            (improvement — per-member publish receipts for multi-crate cuts; verify blind to non-primary member. Likely folds into receipt-provenance above)
    cargo-target-coverage-preflight      (feature — plan-time reject of under-declared plans (fail-fast vs 300s publish-time timeout). ossctl's own contract is correctly declared so it won't trigger for us)
    cargo-metadata-recomputed-per-phase  (improvement — cargo metadata recomputed per phase; cache it)
    cargo-build-disposition-journal      (improvement — journal the per-target build disposition (defer vs package))
    cargo-interleave-real-cargo-integration-test (task — add a real-cargo integration test exercising the interleave end-to-end)
    release-abandon-break-stale-lock     (improvement — `abandon` can't auto-break a stale single-active-cut lock after a hard-kill; shipped stopgap names the lock-file path for manual clearing)
    journal-open-identity-structural-hardening (improvement — validate journal identity/structure on read; needs a corrupt journal, low real-world likelihood)
    homebrew-stable-source-tarball       (improvement — GH auto-archive not byte-stable; build+upload a deterministic source tarball long-term. NOT a blocker)
    homebrew-create-resume-journaling    (task — journal homebrew create sub-steps / reconcile remote)
    homebrew-adapter-fs-port             (task — EffectCtx filesystem-write port for the homebrew create path)
    homebrew-formula-non-rust            (task — generate non-Rust Homebrew formulas; both consumers are Rust today → YAGNI)
    npm-abbreviated-packument            (improvement — npm registry query fetches the full packument; use the abbreviated form. node is not an ossctl target)
    registry-tls-native-certs            (improvement — registry HTTP client uses bundled webpki-roots; optionally also honor the system/native cert store. Deliberate tradeoff for static-musl)
    plan-preimage-projection             (improvement — release/plan: hash a release-relevant projection, not the whole Contract)
    seal-verify-drift-dx                 (improvement — release/plan: ergonomic SEAL_VERSION bump + golden-vector regen. Pair with plan-preimage-projection)
LANE B — contract schema (crates/ossctl-core/src/contract/schema.rs + normalize.rs — SEQUENCE strictly)
  [DONE stint #15] publish-target-none               (done — explicit empty targets:[] honored authoritatively (Option<Vec<Target>>); omitted still expands. No schema_version bump)
  [DONE stint #15] distribution-monorepo-vec         (done — distributions: Vec<Distribution> + per-package association; schema_version 1→2, SEAL 3→4, back-compat deser. Engine single-distribution → per-distribution-release)
  [DONE stint #15] homebrew-tap-contract-consistency-floors (done — cross-field homebrew floors as HARD ERRORS: missing-tap, double-publish, registry/adapter compat + 8-row truth table)
  [DONE stint #15] extra-fields-canonical-json-empty (done — empty extra_fields omitted from canonical JSON (skip_serializing_if) on both Contract + Distribution; SEAL 4→5)
  [DONE stint #15] normalizer-warning-log-injection  (done — diagnostics JSON-encode user-controlled keys (log-injection hardening))
  --- DEFERRED additive hardening ---
    distribution-platforms-adapter-neutral (improvement, DECISION=DEFER — platforms field is Rust-triple-shaped but goreleaser/manual don't consume triples. Revisit at first non-Rust consumer. Jari deferred 2026-08-10)
    distribution-installer-os-classifier (improvement — unify installer/target OS-compat into a structured, adapter-aware classifier (generalizes the landed crosscheck))
    extra-fields-nested-nonstring-yaml   (improvement — extra_fields nested non-string keys collapse in yaml_to_json (never-drop gap))
LANE C — release CI/pipeline infra (.github/workflows/**, dist-workspace.toml — SEQUENCE strictly) — MOSTLY retired for ossctl's own cut; the publish-crates-yml regression is FIXED (stint #16), the rest stays insurance
  [DONE stint #16] publish-crates-yml    (fixed — both cargo publish invocations in the dep-order step now tolerate cargo's exact per-package "already exists on crates.io index" diagnostic as success (anchored match, review-hardened to reject false positives); genuine failures still fail. CI-only change — does not touch the shipped crate)
  [DONE stint #15] publish-crates-release-trigger    (fixed — generated/reference publish-crates.yml triggers on version-tag push, not dead release:published; workflow_dispatch fallback kept. NOTE: this fix surfaced publish-crates-yml above — the tag-push trigger now races the engine's direct publish)
  [CLOSED stint #15] publish-crates-no-auto-trigger  (duplicate of publish-crates-release-trigger)
  [CLOSED stint #15] homebrew-tap-bump-manual-and-missed (obsolete — subsumed by the engine's post-tag direct tap-write)
    release-macos-hauis-coupling         (improvement — the ONE LANE C survivor: cross-platform build is CI-delegated so the engine can't own it. Personal hauis infra → homebase-adjacent; do last / defer)
UNLANED — /oss-* family completeness (skill/template work; no release-engine hot file; run anytime):
    oss-dist-channel-generator           (feature, APPROVED for a future stint — a NEW /oss-* member that generates the distribution channel (dist-workspace.toml + release.yml + tap scaffold + secrets) from a contract's gh-releases/homebrew targets. `ossctl dist generate` does the config half; this wraps it + scaffolds the tap. Build via /worktree-make-skill. Jari approved 2026-08-10)
```
<!-- execution-dag:end -->

## Backlog

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN). `issuectl list` for the
live view. 0.2.5 is shipped; the epic stays open for its tails (see handoff) and the lanes above.
