# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-13 (stint #19). New agent: read this, then continue with a fresh
`/stint-start`. Main is clean + pushed. Live: **0.4.0** on all four channels (UNCHANGED — no release
this round). **⚠️ FIRST THING: a facets-2+3 worker is IN FLIGHT and self-merges** — verify its landing
before anything else (see the ⚠️ block below). Head-of-line stays `release-rust-workspace-multicrate`
(facets 1+4 landed this round; 2+3 in flight)._

_**Stint #19 (2026-08-12→13) — release-engine feature round + 1 feedback round. Landed facets 1+4 of
`release-rust-workspace-multicrate`; facets 2+3 in flight.** Spawned one autonomous design-first spinoff
for the HIGH head-of-line. It landed (green + multi-model `/llm-review`) the two facets that needed no
new architecture decision:_
_- **Facet 1 — dep-ordered multi-crate publish CLOSURE derivation.** `ossctl release plan` now derives
  the full dependency-ordered publish set for a multi-crate Rust workspace (lib before bin) **from a
  bin-only contract** — the core unblock toward retiring hand-cut orchestratectl releases. A live
  `orchestratectl release plan` will now show both crates, ordered._
_- **Facet 4 — `homebrew_tap` carry** from the contract's distribution block into the plan (was null)._
_- **Critical over-publish caught in review before it shipped:** the first impl published EVERY publishable
  workspace member whenever any rust target was present (would sweep an unrelated, deliberately-undeclared
  crate into an IRREVERSIBLE publish). gpt-5.6 flagged it; fixed to only the declared targets' transitive
  dependency CLOSURE, with precise path/workspace edges. (Review report: `history/review-release-rust-
  workspace-multicrate.md`.)_
_Landed head `9a9c219`, +1509 lines across `plan.rs`, `facts/mod.rs`, tests. Green gate clean (439 tests),
pushed._

_**Feedback round — Jari decided facet-2's version source: `--bump major|minor|patch`.** Facet 2 (engine-
owned version bump) collided with the settled 0.3.0 single-source-version decision (`--version` removed):
with no version input the engine can't know what to bump TO. Jari's call: **`--bump major|minor|patch`** —
the engine COMPUTES the new version from the manifest + bump level; NO hand-typed literal, `--version`
stays removed. Recorded in `issues/release-rust-workspace-multicrate/item.md` (commit `b1850ed`)._

_**⚠️ IN-FLIGHT WORKER — verify its landing FIRST (this is why you can't assume main is final):**_
_- Run `01kzx1581x4y9q5jb0ew4daefs` (headless, supervisor was 36114), branch
  `wt/01kzx1581x-release-bump-phase`, building **facets 2+3**: the `--bump` engine-owned version-bump
  plan phase (set `[workspace.package] version`, rewrite `=<ver>` pins, refresh Cargo.lock, finalize
  CHANGELOG) + a **contract-declared `bump_hook`** for version-embedding snapshot regen (schema_version
  bump + back-compat deser). It self-merges via `run merge` and CLOSES the issue if fully done._
_- **NEXT AGENT, do this first:** `orchestratectl run wait 01kzx1581x4y9q5jb0ew4daefs --output json`
  then read `landed`/`summary`; verify by CONTENT on main (not the worker branch ref); run the full
  green gate; `git pull --rebase && git push`. If it landed PARTIAL or `success:false`, read its
  `discussion_items` (`orchestratectl node show 01kzx1581x4y9q5jb0ew4daefs n-0001 --output json`) and
  reconcile. If it landed a schema_version bump, confirm golden vectors updated._

_**After facets 2+3 land:** the issue's ultimate DONE = a real **orchestratectl** cut succeeds end-to-end
(needs crates.io creds + the orchestratectl repo + an IRREVERSIBLE publish) — that live acceptance is the
MAINTAINER's step, not a worktree's. Only then close/retire the head-of-line._

_**Hardening backlog from facet-1 review** lives inside `issues/release-rust-workspace-multicrate/item.md`
(NOT yet separate issuectl issues): workspace-graph parser edge cases — `workspace.dependencies`-inherited
renames, publish-field workspace inheritance, non-virtual root-package workspaces, multi-line inline
tables, recursive/patterned member globs, Windows abs-path confinement, manifest-truncation-as-
authoritative, and a cut-time cross-check of the sealed plan ORDER vs `cargo metadata` topology. File as
issues + lane them if picked up._

_**Housekeeping:** the Dependabot `clap` PR is still open on the remote — adjacent, not triaged._

_🎉 **THREE RELEASES THIS SESSION — 0.2.5, 0.3.0, 0.4.0.** Stint #16 made the engine's real-cut publish
TRUSTWORTHY (0.2.5) then HARDENED it (0.3.0, BREAKING). Stint #17 shipped 0.4.0 — pi.dev skill dual-homing._

_**0.4.0 (2026-08-11, stint #17)** — `pidev-dual-home-skills` DONE: `ossctl skill install` now dual-homes
each `SKILL.md` into `~/.pi/agent/skills/<name>/` (pi.dev harness) — with `--agent` omitted it writes
BOTH Claude + pi.dev by DEFAULT (`--agent claude`=single-home, `pi`/`codex`=narrow, `all`=every runtime).
New `Runtime::Pi`; `--json installed[]` shape unchanged (additive). Part of the Claude Code→pi.dev
migration (homebase `pidev-migration`/WS4). Minor bump (new feature + default-behavior change). Fully-
autonomous engine self-cut, exit 0, all four channels live. Plan `b77d521c…`, head `f5b31dc`._

_🎉 **STINT #16 — 0.2.5 THEN 0.3.0 SHIPPED. The engine's real-cut publish is now TRUSTWORTHY and
HARDENED.** Round-1 fixed the two HIGH bugs the first real downstream cut (issuectl 0.8.1) exposed and
shipped 0.2.5 (self-visibility confirm + single-source version). Round-2 (Jari approved doing ALL 4
cut-noop review follow-ups) shipped 0.3.0 — a BREAKING release removing --version + 3 more hardenings.
Both were fully-autonomous engine self-cuts, exit 0, all four channels verified live._

_**0.3.0 (2026-08-11, BREAKING)** — 4 cut-noop review follow-ups, 3 sequenced LANE A workers:
`release-drop-version-flag` (HARD-removed --version — version derives solely from the manifest; stray
flag = unknown_flag error; all callers + recipe + oss-release skill updated), `version-source-fail-
closed-nonrust` (VersionSource capability model — manifest-versioned non-Rust now fails CLOSED, was
open), `release-cut-clean-checkout` (cut/resume publish from a fresh checkout of the sealed head_sha —
reproducible, immune to mid-cut edits; proven on the 0.3.0 self-cut), `is-published-digest-authenticate`
(checksum-match the resume idempotency skip before trusting it; mismatch/outage fail closed). Plan
`7a38b4ee…`, head `da9850e`. ⚠️ Worker B (clean-checkout) HUNG on the first spawn (agent alive, 0
commits/events for 6h) → cancelled (nothing to harvest) + re-spawned fresh, landed clean 2nd try._

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

_✅ **`pidev-dual-home-skills` is DONE (shipped in 0.4.0)** — no longer the head-of-line. (Note: the
earlier PENDING runs named `pidev-dual-home-skills` / `dual-home-skills` turned out to be **glasspad's**
worker, not ossctl's; ossctl's was implemented fresh this stint.)_

_⚠️ **ONE HIGH blocker is now queued: `release-rust-workspace-multicrate`** (feature, LANE A head-of-line,
filed 2026-08-12 by the orchestratectl session). The engine produces an INCOMPLETE plan for a DOWNSTREAM
two-crate Rust workspace: (1) only the bin crate is a target → a cut fails on the `=<ver>` pin because
the lib isn't published (must DERIVE dep-ordered member publish from the workspace graph); (2) no
version-bump phase (the engine must OWN the bump — workspace version + `=<ver>` pins + Cargo.lock +
CHANGELOG finalize + regenerate version-embedding insta snapshots — as a content-addressed plan phase);
(3) `homebrew_tap` null despite the contract's distribution declaring the tap (carry it into the plan).
DONE = a real orchestratectl cut succeeds end-to-end. RELATED downstream-cut gaps (all still DEFERRED):
`release-ci-publish-mode` (glasspad friction — 'publish-in-CI / tag-only cut' mode; still wants Jari's
triage) and `per-distribution-release`. ossctl's OWN cut path stays trustworthy + hardened (0.2.5/0.3.0),
so Track B is unaffected; this is purely a downstream-consumer gap. Everything else in the DAG is
DEFERRED/optional._

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
0.2.4 + cleared all decisions. #16 shipped 0.2.5 (real-cut publish trustworthy) THEN 0.3.0 (BREAKING:
--version removed + non-Rust fail-closed + clean-checkout cut + digest-authenticated resume skip). #17
shipped 0.4.0 (skill install dual-homes into pi.dev). #18 was a short listing/DAG-maintenance round (no
release, no code) that reconciled the new HIGH `release-rust-workspace-multicrate` into the DAG. Epic
`ossctl-phase4-build` stays OPEN. Cross-repo standardisation + hauis infra remain HOMEBASE concerns
(homebase issue `cross-repo-release-standardisation`), NOT ossctl work. ---_

**Read first (the spec):** `docs/adr/000{1,2,3,4}-*.md` (CLI taxonomy, release engine, config+journal, one-target-one-publish-unit).

## Execution DAG (2026-08-13, stint #19 handoff)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/handoff (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

The cross-repo standardisation ("Track A") and hauis CI runners are HOMEBASE concerns (Jari's
personal environment), NOT ossctl work — moved to homebase issue `cross-repo-release-standardisation`.
Do not re-add them here.

**Track B — "ossctl cuts ITSELF through the engine" — ✅ COMPLETE (stint #14) and now ROUTINE (stints
#15/#16/#17 shipped 0.2.4/0.2.5/0.3.0/0.4.0 the same way; #16 made the real-cut publish trustworthy + hardened).**
⚠️ **ONE HIGH release feature, PARTIALLY LANDED (stint #19):** `release-rust-workspace-multicrate` —
made the engine's plan COMPLETE for a downstream two-crate workspace. Facets 1 (dep-ordered multi-crate
publish CLOSURE derivation from a bin-only contract) + 4 (homebrew_tap carry) LANDED; facets 2 (`--bump`
engine-owned version-bump phase) + 3 (contract-declared snapshot bump_hook) are IN FLIGHT (run
01kzx1581x4y9q5jb0ew4daefs, self-merges — verify FIRST). After 2+3 land, ultimate DONE = a real
orchestratectl cut end-to-end (maintainer's step). Everything ELSE below is DEFERRED hardening,
review follow-ups, or the one approved future feature (`oss-dist-channel-generator`, UNLANED). LANE C
is retired for ossctl's own cut — only `release-macos-hauis-coupling` survives (homebase-adjacent).
**Do NOT harden LANE C.**

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: release-rust-workspace-multicrate (HIGH, feature — STILL the head; PARTIALLY LANDED stint #19). Facets 1 (dep-ordered multi-crate publish CLOSURE derivation from a bin-only contract) + 4 (homebrew_tap carry) LANDED green + reviewed (head 9a9c219). Facets 2 (`--bump` engine-owned version-bump phase) + 3 (contract-declared bump_hook for snapshot regen) are IN FLIGHT in run 01kzx1581x4y9q5jb0ew4daefs (self-merges — verify its landing FIRST, see handoff ⚠️). Maintainer decision recorded: facet-2 version source = `--bump major|minor|patch` (engine computes; --version stays removed). After 2+3 land, ultimate DONE = a real orchestratectl cut end-to-end (maintainer's step). 0.4.0 stays live on all four channels; Track B (ossctl's own cut) routine.
  Then DEFERRED/optional: release-ci-publish-mode (glasspad friction — needs Jari triage; RELATED — both are downstream-cut gaps), per-distribution-release, release-verify-delegated-github-release, release-cut-stale-binary-guard, the cargo receipt-provenance cluster, LANE B additive hardening. FEATURE (approved, own stint): oss-dist-channel-generator via /worktree-make-skill. Also open in orchestratectl repo: supervisor-stall-detection (filed this session).
LANE A — release engine (crates/ossctl-core/src/release/**; SEQUENCE strictly)
  ▶ release-rust-workspace-multicrate  (HIGH, feature — PARTIALLY LANDED stint #19; still the head. [DONE facet 1] DERIVE dep-ordered multi-crate publish CLOSURE from a bin-only contract (lib → bin) — landed green + reviewed, gpt-5.6 caught+fixed a critical over-publish (was publishing every member; now only the declared targets' transitive closure). [DONE facet 4] carry homebrew_tap into the plan. [IN FLIGHT facets 2+3 — run 01kzx1581x4y9q5jb0ew4daefs, self-merges] facet 2 = `--bump major|minor|patch` engine-owned version-bump phase (compute version from manifest+level; set `[workspace.package] version`, rewrite `=<ver>` pins, Cargo.lock, CHANGELOG finalize — maintainer decision recorded in item.md, commit b1850ed); facet 3 = contract-declared `bump_hook` for version-embedding snapshot regen (schema_version bump + back-compat deser). After 2+3: ultimate DONE = a real orchestratectl cut end-to-end (maintainer's step). Facet-1 review left a workspace-graph-parser hardening backlog inside item.md. collision: contract/schema.rs touched for facet-3 field)
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
