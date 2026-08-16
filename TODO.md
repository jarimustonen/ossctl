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
GLOBAL HEAD-OF-LINE: issue triage cleanup completed 2026-08-16. Every open non-epic issue is now either laned below or intentionally closed as obsolete, duplicate, or wontfix.

LANE release-safety
  ▶ release-verify-delegated-github-release
    release-ci-publish-mode
    release-cut-stale-binary-guard
    contract-validate-warn

LANE release-hardening
  ▶ cargo-publish-receipt-provenance-resume-safety
    homebrew-publish-resume-idempotency
    release-abandon-break-stale-lock

LANE contract-safety
  ▶ extra-fields-nested-nonstring-yaml

LANE oss-family
  ▶ oss-dist-channel-generator
```
<!-- execution-dag:end -->

## Backlog

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN). `issuectl list` for the
live view. 0.4.0 is shipped; the epic stays open for its tails (see handoff) and the lanes above.

## Piialiisan bugiraportit

- Intake bugs reviewed 2026-08-16. `intake-bug-ossctl-878b3a0790a5` closed fixed because current `release plan` supports `--json`; `intake-feature-ossctl-04e19af4e11d` closed duplicate into `oss-dist-channel-generator`.
