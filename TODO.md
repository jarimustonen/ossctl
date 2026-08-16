# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-14 (stint #20). New agent: read this, then continue with a fresh
`/stint-start`. Main is clean + pushed. Live: **0.5.0** on all four channels (SHIPPED this round).
**No active/queued task — the DAG frontier is EMPTY.** The HIGH head `release-rust-workspace-multicrate`
is CLOSED done; everything else open is DEFERRED hardening / review follow-ups / optional features. The
only live thread is external: the **orchestratectl 0.2.0 `--bump` live-acceptance cut** (orchestratectl's
timeline, prepared this round — see below). A fresh `/stint-start` will find nothing queued; wait for
Jari to name work, or pick a DEFERRED item if he wants backlog burn-down._

_**Stint #20 (2026-08-13→14) — shipped 0.5.0, closed the HIGH head, prepared orchestratectl for its live cut.**_

_**🎉 0.5.0 SHIPPED (2026-08-13) — all four channels, autonomous engine self-cut, exit 0.** Plan
`83dc4e29…`, release commit `1be179b`, tag `v0.5.0`. crates.io ×2 (ossctl-core + ossctl via sparse
index), GitHub Release v0.5.0 + 14 cross-platform assets (cargo-dist CI, **hauis macOS aarch64 clean, no
400**), Homebrew tap → v0.5.0 (engine direct tap-write, real sha256). Cut via the PROVEN recipe (manual
version bump in step 1, then `release plan`/`cut` WITHOUT `--bump`) — deliberately NOT dogfooding the new
`--bump` executor on the irreversible path (its live acceptance is decoupled, see orx below). **What 0.5.0
ships (everything since 0.4.0):** the full `release-rust-workspace-multicrate` feature — facet 1
(dep-ordered multi-crate publish CLOSURE from a bin-only contract), facet 4 (`homebrew_tap` carry), facets
2+3 (`--bump major|minor|patch` plan phase + cut-time executor + contract `release.bump_hook`)._

_**✅ `release-rust-workspace-multicrate` CLOSED done (`9876bd4`).** All 4 facets landed across stints
#19–#20 (2 autonomous spinoffs, each green + 4-model `/llm-review`): the plan side (`bff2cd2`+`31e3df7`,
stint #19 in-flight worker that landed this round) and the cut-time EXECUTOR (`ce33e25`+`7590c2a`, this
round — executes the sealed bump in the clean checkout: version + precise `=`-only pin rewrites via Facts
dep-req strings + Cargo.lock + CHANGELOG finalize + `bump_hook`; tag on the BUMP commit; resume-safe
`BumpApplied` guard so no double-bump; journal schema v3→v4; a fail-closed `bump_hook` exec contract
`sh -c` verbatim + post-hook version validation). The review caught + fixed a unanimous critical (resume
built/published the PRE-bump tree). Maintainer closed it **on code-complete**: the original "verified on a
real cut" acceptance is a live, credential-gated validation, **decoupled** to the orx cut below — if that
surfaces problems they become NEW issues (or this reopens), not open work here._

_**🔗 THE ONE LIVE THREAD — orchestratectl 0.2.0 `--bump` live-acceptance cut (orx's timeline).** This is
the real live proof of the `--bump` executor. orchestratectl (`~/Sources/orchestratectl`) is mid-refactor
toward **0.2.0** (bigger refactoring in progress; nothing user-facing was releasable when checked —
`[Unreleased]` empty). We did NOT cut a contentless release. Instead we PREPARED orx (autonomous spinoff
in the orx repo, `2db04f4`+`63ad5bf`, validated with the ossctl 0.5.0 binary):_
_- declared `release.bump_hook: "INSTA_UPDATE=always cargo test -p orchestratectl --test envelope_snapshots"`
  (dependency-free; regenerates ONLY the 3 version_* insta snapshots on a bump, never the version — passes
  the executor's fail-closed post-hook guard);_
_- added a v2 `distribution:` block to orx's `OSS-RELEASE.md` (tap `jarimustonen/homebrew-orchestratectl`
  + platforms) — **because ossctl 0.5.0 sources the plan's `homebrew_tap` from the contract's distribution
  block, NOT from `dist-workspace.toml`** (a real downstream-readiness gap this prep surfaced);_
_- proven: scratch bump 0.1.8→0.2.0 regenerated exactly the 3 snapshots + `check-version-snapshots.sh` +
  `cargo test --workspace` green; and ossctl-0.5.0 `contract validate` (0 warn) + `release plan --bump
  minor` → version 0.2.0, both crates dep-ordered (octl-core → orchestratectl), tap carried, bump_hook
  surfaced verbatim._
_**NEXT for this thread (whenever orx 0.2.0 is ready to ship — likely a separate orx stint, not ossctl):**
run `ossctl release cut --bump minor` on orx with a 0.5.0+ binary. If it works, the `--bump` executor is
live-proven and hand-cutting orchestratectl retires. If problems: file NEW issues (orx or ossctl as
appropriate). NB: the PATH `ossctl` is stale (0.2.2 via brew) — `brew upgrade ossctl` to 0.5.0 or use a
fresh-built binary; `ossctl version` is the only stale-binary tell._

_**Housekeeping:** no lingering ossctl worktrees (both round workers settled + torn down). The Dependabot
`clap` PR (now `clap-4.6.6`) is still open on the remote — adjacent, not triaged._

_**Earlier releases (compressed):** 0.4.0 (#17, pi.dev skill dual-home), 0.3.0 (#16r2, BREAKING —
--version removed + non-Rust fail-closed + clean-checkout cut + digest-authenticated resume skip), 0.2.5
(#16, real-cut publish made trustworthy: post-publish self-visibility confirm + single-source version).
Full detail in git log + `issues/`._

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
0.2.4 + cleared all decisions. #16 shipped 0.2.5 (real-cut publish trustworthy) THEN 0.3.0 (BREAKING:
--version removed + non-Rust fail-closed + clean-checkout cut + digest-authenticated resume skip). #17
shipped 0.4.0 (skill install dual-homes into pi.dev). #18 was a short listing/DAG-maintenance round (no
release, no code) that reconciled the new HIGH `release-rust-workspace-multicrate` into the DAG. Epic
`ossctl-phase4-build` stays OPEN. Cross-repo standardisation + hauis infra remain HOMEBASE concerns
(homebase issue `cross-repo-release-standardisation`), NOT ossctl work. ---_

**Read first (the spec):** `docs/adr/000{1,2,3,4}-*.md` (CLI taxonomy, release engine, config+journal, one-target-one-publish-unit).

## Execution DAG (2026-08-14, stint #20 handoff)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/handoff (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

The cross-repo standardisation ("Track A") and hauis CI runners are HOMEBASE concerns (Jari's
personal environment), NOT ossctl work — moved to homebase issue `cross-repo-release-standardisation`.
Do not re-add them here.

**Track B — "ossctl cuts ITSELF through the engine" — ✅ COMPLETE (stint #14) and ROUTINE (stints
#15–#20 shipped 0.2.4→0.5.0 the same way).** ✅ **The HIGH release feature is now DONE (stint #20):**
`release-rust-workspace-multicrate` — all 4 facets landed + CLOSED; shipped in 0.5.0. **The active
frontier is EMPTY** — no queued/scheduled task. Everything below is DEFERRED hardening, review
follow-ups, or the one approved future feature (`oss-dist-channel-generator`, UNLANED). The one live
thread is EXTERNAL: the orchestratectl 0.2.0 `--bump` live-acceptance cut (orx's timeline — see handoff
block; prepared this round). LANE C is retired for ossctl's own cut — only `release-macos-hauis-coupling`
survives (homebase-adjacent). **Do NOT harden LANE C.**

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: (NONE — active frontier EMPTY). The HIGH `release-rust-workspace-multicrate` is DONE + CLOSED (all 4 facets, shipped 0.5.0). No task is queued or scheduled. A fresh /stint-start finds nothing to pick — wait for Jari to name work, or burn down a DEFERRED item below if he asks. The one live thread is EXTERNAL, not an ossctl DAG node: the orchestratectl 0.2.0 `--bump` live-acceptance cut (orx prepared this round; runs on orx's timeline — `ossctl release cut --bump minor` there when 0.2.0 ships).
  DEFERRED/optional (none blocks anything, none scheduled): release-ci-publish-mode (glasspad friction — needs Jari triage; RELATED to the orx downstream-cut work), per-distribution-release, release-verify-delegated-github-release, release-cut-stale-binary-guard, the cargo receipt-provenance cluster, LANE B additive hardening. FEATURE (approved, own stint): oss-dist-channel-generator via /worktree-make-skill.
LANE A — release engine (crates/ossctl-core/src/release/**; SEQUENCE strictly)
  [DONE stint #20] release-rust-workspace-multicrate  (HIGH, feature — CLOSED done, shipped 0.5.0. facet 1 = dep-ordered multi-crate publish CLOSURE from a bin-only contract (lib → bin; gpt-5.6 caught+fixed a critical over-publish); facet 4 = homebrew_tap carry; facet 2 = `--bump major|minor|patch` engine-owned version-bump plan phase + cut-time EXECUTOR (version + `=`-only pin rewrites via Facts dep-reqs + Cargo.lock + CHANGELOG finalize + bump_hook; tag on the bump commit; resume-safe BumpApplied guard; journal v3→v4); facet 3 = contract-declared `release.bump_hook` for version-embedding snapshot regen. 2 spinoffs, each green + 4-model /llm-review (a unanimous critical — resume built the PRE-bump tree — caught+fixed). Live `--bump` acceptance decoupled to the orx 0.2.0 cut. Facet-1 review left a workspace-graph-parser hardening backlog inside item.md — file as issues if picked up.)
  [DONE stint #15] resume-publish-phase-never-reached      (fixed — resume no longer demands --allow-unverified when the publish phase was never reached; unsafe rows unchanged)
  [DONE stint #15] release-abandon-reason-leading-dashes   (fixed — `release abandon --reason` accepts values starting with `--` via allow_hyphen_values)
  [DONE stint #15] homebrew-tapwrite-preserve-formula      (done — ownership marker: full-regen only when marked, else surgical url/sha edit or fail-closed refusal; hand-maintained formulas preserved)
  [DONE stint #16] release-cut-publish-noop (fixed — landed a --version-vs-tree-manifest drift guard on plan/cut/resume + a mock-registry cut integration test asserting versions actually land with a per-member receipt + root-cause analysis. NOT a reproduced source no-op: worker judged the issuectl timeout most consistent with an env/registry-token difference OR issuectl-core not declared as its own target — not reproducible locally w/o crates.io creds. REAL fix (post-publish self-visibility check) deferred to cut-noop-self-visibility-check as a maintainer decision (behavior change on the irreversible phase))
  [DONE stint #16] cut-noop-self-visibility-check (fixed — after cargo publish the adapter confirms the target's OWN {name,version} reached the crates.io index (bounded index-wait) before journaling a receipt; a silent no-op fails the cut closed, an outage fails closed distinctly. Proven live on the 0.2.5 self-cut. 4-model /llm-review applied)
  [DONE stint #16] release-version-single-source (done — release version derived from the workspace manifest (single source of truth); --version is now an optional must-match confirmation that subsumes the stint #16 drift guard. plan.rs/release.rs)
  --- DONE stint #16 round-2 (all 4 cut-noop review follow-ups; shipped in 0.3.0, 2026-08-11) ---
  [DONE stint #16r2] release-drop-version-flag            (done — HARD-removed --version from release plan/cut; version derives solely from the manifest, a stray flag is an unknown_flag error. All in-repo callers + AGENTS.md recipe + oss-release skill updated. BREAKING → 0.3.0)
  [DONE stint #16r2] version-source-fail-closed-nonrust   (done — VersionSource capability model keyed on Ecosystem: rust/node/python=Manifest fail-closed on missing version; go/binary=Distribution skip. No longer fails OPEN for manifest-versioned non-Rust)
  [DONE stint #16r2] release-cut-clean-checkout           (done — cut/resume publish from a fresh git-worktree checkout of the sealed plan.head_sha; fail-closed if the commit is absent; journal+tag stay on the real repo. Proven on the 0.3.0 self-cut)
  [DONE stint #16r2] is-published-digest-authenticate     (done — digest-authenticate the resume idempotency skip: RegistryQuery exposes published_checksum (sparse-index cksum), repackage+hash the target .crate, compare — match trusts+records, mismatch=DigestMismatch fail-closed, outage=RegistryUnavailable fail-closed. 4-model review: the DEFINITIVE cross-toolchain-safe form = journal the intended digest at publish time → folded into cargo-publish-receipt-provenance-resume-safety)
  --- DEFERRED hardening + review follow-ups (none blocks anything) ---
    release-ci-publish-mode              (feature — filed by ANOTHER session cutting glasspad 0.4.0 (2026-08-10). Engine only does a LOCAL cargo publish in publish-all, but some repos (glasspad) forbid it — publish is CI-triggered by the tag push (publish-crates.yml + release.yml); local ~/.cargo creds may be stale → 403. Needs a contract field + a 'tag-only cut' mode that SKIPS local publish and delegates to CI. Spans LANE B (schema field) + LANE A (engine). collision: contract/schema.rs. NOT in the round-2 scope — surfaced from real glasspad friction; triage with Jari)
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
    contract-validate-warn               (improvement — filed stint #20 from orx prep. `release plan` sources homebrew_tap from the contract distribution block, NOT dist-workspace.toml; a repo declaring the tap only in dist-workspace.toml gets a SILENT null tap → cut drops the homebrew leg. `contract validate` should WARN on that drift (cross-read dist-workspace.toml; warning not error, contract stays authoritative). Non-blocking, additive, no schema change)
LANE C — release CI/pipeline infra (.github/workflows/**, dist-workspace.toml — SEQUENCE strictly) — MOSTLY retired for ossctl's own cut; the publish-crates-yml regression is FIXED (stint #16), the rest stays insurance
  [DONE stint #16] publish-crates-yml    (fixed — both cargo publish invocations in the dep-order step now tolerate cargo's exact per-package "already exists on crates.io index" diagnostic as success (anchored match, review-hardened to reject false positives); genuine failures still fail. CI-only change — does not touch the shipped crate)
  [DONE stint #15] publish-crates-release-trigger    (fixed — generated/reference publish-crates.yml triggers on version-tag push, not dead release:published; workflow_dispatch fallback kept. NOTE: this fix surfaced publish-crates-yml above — the tag-push trigger now races the engine's direct publish)
  [CLOSED stint #15] publish-crates-no-auto-trigger  (duplicate of publish-crates-release-trigger)
  [CLOSED stint #15] homebrew-tap-bump-manual-and-missed (obsolete — subsumed by the engine's post-tag direct tap-write)
    release-macos-hauis-coupling         (improvement — the ONE LANE C survivor: cross-platform build is CI-delegated so the engine can't own it. Personal hauis infra → homebase-adjacent; do last / defer)
UNLANED — skill-installer + /oss-* family (skill/template work; no release-engine hot file; run anytime):
  [DONE stint #17] pidev-dual-home-skills (done — SHIPPED in 0.4.0, 2026-08-11. `ossctl skill install` now dual-homes: new Runtime::Pi writes SKILL.md into ~/.pi/agent/skills/<name>/; with --agent omitted the installer writes BOTH Claude + pi.dev by DEFAULT (--agent claude=single-home, pi/codex=narrow, all=every runtime). --json installed[] shape unchanged (additive). glasspad-consistent design. Green gate + /llm-review)
    oss-dist-channel-generator           (feature, APPROVED for a future stint — a NEW /oss-* member that generates the distribution channel (dist-workspace.toml + release.yml + tap scaffold + secrets) from a contract's gh-releases/homebrew targets. `ossctl dist generate` does the config half; this wraps it + scaffolds the tap. Build via /worktree-make-skill. Jari approved 2026-08-10)
```
<!-- execution-dag:end -->

## Backlog

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN). `issuectl list` for the
live view. 0.4.0 is shipped; the epic stays open for its tails (see handoff) and the lanes above.

## Piialiisan bugiraportit

- [ ] 🐛 Piialiisan bugiraportti: release plan rejects --output flag though other subcommands accept it — jari via Telegram ([`intake-bug-ossctl-878b3a0790a5`](issues/intake-bug-ossctl-878b3a0790a5/item.md))
