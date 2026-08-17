# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)

_Handoff written 2026-08-17 (stint #22, Fable-orchestrated). New agent: read this, then continue
with a fresh `/stint-start`. Main is clean + pushed. Live: **0.7.0 on all four channels,
independently verified** (crates.io ×2, GitHub Release 11 assets, tap formula 0.7.0 + marker,
local `brew` install runs). **GLOBAL HEAD-OF-LINE: `homebrew-ci-delegated-adapter`** (deliberate
resequence: it unlocks fleet-wide contract uniformity for the four public repos with less work
than `release-ci-publish-mode`, which now sits second in the lane). Second parallel lane head:
`intake-bug-ossctl-d9b2ec7bb6d9` (maintainer-acked 2026-08-17: `--version` must alias the
`version` verb; note project-canon filed the same class as its own `version-flag-alias`)._

_**🚀 0.7.0 (2026-08-17, run `01M07BDD2RBRADH70DYR41JPBA`) — the first `--bump` engine cut, and
the first verified one.** Seven phases green: bump (0.6.1→0.7.0 by the executor's first live run:
version + `=`-pin + Cargo.lock + CHANGELOG finalize + commit `1294b2ef`) → dry-run → build →
publish (crates.io ×2, gh-releases delegated) → tag → dist (tap-write) → **verify (all four
targets OBSERVED — the new barrier's maiden run)**. The cut was invoked WITHOUT `--bump`,
proving the plan store recovers the bump disposition — the exact issuectl 0.14.1 failure shape,
now fixed in production. Post-cut: main fast-forwarded to the bump commit and pushed._

_**Stint #22 (2026-08-17) — the release-safety cluster is CLOSED as one design.** Stint #21's ⭐
THEME (the engine reports success without verifying the artifact) plus the two HIGH field bugs
were resolved by one design doc (`issues/release-tag-preempts-cargo-dist/design.md`, D1–D6) and
six worktree units, all landed + green:_
_- **D1+D2 plan store** — `release plan` persists the sealed plan under
  `git-common-dir/ossctl/plans/<plan_id>.json`; `cut` recovers an omitted `--bump` from it
  (field-verified: the exact issuectl repro now cuts), `resume` survives a code fix moving HEAD,
  and `plan_stale` never again recommends a republishing plan id. ⚠️ Mechanism correction: the
  `--bump` staleness hash was NEVER broken — the operator had to repeat `--bump` at cut and the
  error steered them to the no-bump plan. Closed: `release-bump-plan-uncuttable`,
  `resume-drift-after-fix`._
_- **D3 mandatory verify phase** — a new final coordinator barrier: a run is `Completed` only
  when every target is OBSERVED at its destination (crates.io index, tap formula content,
  GitHub Release assets; journal v4→v5; `Unknown` is not green). Closed:
  `release-verify-homebrew-tap`, `release-verify-delegated-github-release`,
  `release-tag-preempts-cargo-dist`._
_- **D4 undeclared-distribution refusal** — facts detect cargo-dist/tag-workflows; `cut` refuses
  `undeclared_distribution` before creating a run when the contract under-declares (the issuectl
  0.14.1 accident shape). **D5** stale-lock recovery in `abandon` (closed
  `release-abandon-break-stale-lock`). **D6** hermetic e2e harness
  (`crates/ossctl-cli/tests/e2e/`) — the cut is now testable without cutting. Plus cli-canon §2
  exit codes (closed `cli-canon-exit-codes`)._

_**🌍 CROSS-REPO AUDIT (maintainer request) — full report:
`homebase/issues/cross-repo-release-standardisation/audit-2026-08-17.md`.** The fleet had FOUR
release doctrines; contracts normalized + pushed in issuectl (Windows dropped, gh-releases
declared), orchestratectl (gh-releases declared), project-canon (orphaned tap fixed: cargo-dist
homebrew publisher enabled — its token had existed since 08-15), glasspad (engine-owned homebrew
target removed — it was a double-writer landmine). intakectl is deliberately publish-none
(deploy.sh → haapa). New issues filed here: `homebrew-ci-delegated-adapter` (lets the other
repos declare their CI-owned tap; also fixes `dist generate` under-generating),
`publish-none-unrepresentable` (intakectl's blocker), plus audit evidence on
`release-ci-publish-mode` (glasspad forbids engine publish — this mode is its path onto the
engine)._

_**⚠️ MARKER GUIDANCE CORRECTED.** The old instruction "prepend the ossctl ownership marker to
every tap before cutting" applies ONLY to engine-written taps (ossctl's own). The
glasspad/orchestratectl/issuectl/project-canon taps are written by cargo-dist CI on every tag —
a marker there is pointless and would vanish on the next release. Do not add markers to them._

_**Worker-model note (for the orchestrator).** A pi/gpt-5.6-terra worker gave up twice on the
verify-phase unit (large semantic seam). Salvage-commit + a fresh worker on gpt-5.6-sol (pi
default model temporarily switched via `~/.pi/agent/settings.json`, restored after spawn)
finished it cleanly. For coordinator-seam units, prefer the stronger model up front._

_**Stint #21 (2026-08-16→17) — shipped 0.6.0 + 0.6.1, unbroke Homebrew, pruned 40% of the issue base.**_

_**🍺 THE HOMEBREW LEG WAS BROKEN AND IS NOW FIXED.** `brew upgrade ossctl` had failed silently since
0.2.3: the engine-written formula ran `cargo install` against a virtual workspace manifest, which
cannot work. Six releases reported their Homebrew leg green while publishing a formula nobody could
install; the maintainer's machine sat on **0.2.2** and it was mistaken for a stale-install habit. The
formula now carries an ownership marker, per-platform prebuilt archive URLs with checksums,
`bin.install`, and no Rust toolchain dependency. **Verified: `brew upgrade ossctl` 0.2.2 → 0.6.1.**_

_**⚠️ ~~EVERY OTHER TAP NEEDS A ONE-TIME MANUAL MARKER~~ — SUPERSEDED by the stint #22 audit.**
The issuectl / glasspad / orchestratectl / project-canon taps are written by **cargo-dist CI**
on every tag, not by the engine's tap-write, so the ossctl ownership marker does not apply to
them (it would vanish on the next release). The marker concerns ONLY engine-written taps
(ossctl's own, already marked). Their contracts also no longer declare engine-owned homebrew
targets, so an engine cut cannot reach the tap-write refusal there. See the #22 handoff block
and `homebase/issues/cross-repo-release-standardisation/audit-2026-08-17.md`._

_**Releases: 0.6.0 then 0.6.1, both engine-cut.** 0.6.0 (`ffcef9c`) shipped the installable formula,
the downstream-safe stale-binary guard, `config path`/`config show`, the contract never-drop fix, the
`/oss-dist` skill, and the cargo-dist tap warning — but its dist phase failed on the Windows platform
entry, so the tap never got it. 0.6.1 (`2846d66`, run `01M0727VBSV1P8N6YHC0PWBVXQ`) fixed that and
completed the tap. **The engine stopped three times before getting through, and every stop was
correct** — a platform Homebrew cannot serve, a sealed plan that no longer matched the tree, and an
unmarked formula it refused to clobber. Nothing wrong was written anywhere. Contrast with the six
silent green releases that preceded them._

_**🪟 WINDOWS DROPPED (maintainer decision, 2026-08-17).** ossctl no longer builds or ships a Windows
binary or a PowerShell installer. No default changed — Windows has never been in the normalizer's
default platform set (it is a documented deliberate omission); this repo had simply opted in.
**`issuectl` is the only remaining repo with that opt-in live** — remove it in its next cut._

_**🧹 40% OF THE ISSUE BASE WAS SPECULATIVE AND IS GONE (maintainer decision).** A full sweep found
that roughly two in five open issues were AI-review output with no observed failure behind them:
cosmic-ray scenarios, checks duplicating checks elsewhere, hostile-input hardening on paths where the
only actor is the maintainer. Five were closed `wontfix` (the cargo publish digest-provenance cluster
and the homebrew resume remnants), each with a written reason **and a reopen condition** so the same
finding is not re-filed next quarter. `release-abandon-break-stale-lock` was kept but **narrowed** —
its real, field-confirmed core is in scope; its process-id-reuse and network-filesystem defences are
not. **Judge an issue by whether its failure can actually occur here; use provenance only as a
supporting signal.** Two units this round were spent on findings of this class before the pattern was
spotted._

_**📮 FILED IN OTHER REPOS this round (this repo owns none of them):**_
_- `homebase/triage-catch-review-slop` — teach `/triage-unlaned-issues` to catch the speculative class above._
_- `homebase/triage-script-json-envelope` — **the unlaned-issue detector is BROKEN**: it reads the
  pre-envelope `issuectl dag --json` shape, exits 3, and `/wrap-up` treats that as "clean". Two real
  issues went undetected this round. Until it is fixed, compute the set by hand:
  `comm -3 <(issuectl --json ls --status open | jq -r '.data[]|select(.type!="epic")|.slug'|sort -u) <(issuectl dag --json | jq -r '.data.lanes[].issues[].slug'|sort -u)`_
_- `orchestratectl/worktree-issue-provenance` — worktree-filed issues must land UNLANED with visible AI-review provenance._
_- `project-canon/canon-verify-deferrals` — a deferral justification must be verified, not inherited.
  Root case: a config comment claimed Homebrew publishing was blocked on a token that had existed for
  **three and a half months**, and pointed at an owning issue that did not exist. The tap sat three
  releases behind. Each later pass read the comment and built around it._
_- `issuectl/homebrew-tap-stale` — annotated with the token evidence; its blocker was never real._

_**Release queue (recommended order, and why):** 1. ~~ossctl 0.6.1~~ DONE. 2. ~~issuectl 0.14.1~~ DONE
(cut by the maintainer; it produced the two head bugs). 3. **glasspad 0.15.0** — four unreleased
feat/fix commits; distribution healthy; **its contract declares both the gh-releases/cargo-dist and
homebrew targets, so it is NOT exposed to `release-tag-preempts-cargo-dist`** — but it does still need
the tap marker above. **No release needed:** orchestratectl and project-canon are current everywhere._

_**issuectl's contract under-declares its distribution** — only two crates.io targets, no gh-releases
and no homebrew target, though the repo really uses both. That is the trigger for
`release-tag-preempts-cargo-dist` and why its tap went stale. Fixing that contract is an issuectl-repo
task, not ossctl's._

_**Housekeeping:** no lingering ossctl worktrees. Two workers were cancelled mid-round (one wedged, one
returned an unlanded diff on an over-large unit) — the unit was then **split, and the split was itself
reverted** when the whole cluster turned out to be speculative. The Dependabot `clap` PR is still open,
untriaged. The `--bump` executor is confirmed **broken end-to-end** (`release-bump-plan-uncuttable`);
stint #20's decision not to dogfood it on an irreversible path was vindicated._

_**Stint #20 (2026-08-13→14) — shipped 0.5.0.** Cut via the manual-bump recipe, deliberately NOT
dogfooding the new `--bump` executor on the irreversible path; that decision is now vindicated (see
`release-bump-plan-uncuttable`). Closed `release-rust-workspace-multicrate` on code-complete (all 4
facets: dep-ordered multi-crate closure from a bin-only contract, `homebrew_tap` carry, the `--bump`
plan phase + cut-time executor + `release.bump_hook`). Also prepared orchestratectl for a `--bump`
live-acceptance cut — that thread is now MOOT: `--bump` is broken end-to-end and must be fixed before
any repo can use it. Full detail in git log + `issues/`._

_**Earlier releases (compressed):** 0.5.0 (#20, dep-ordered multi-crate closure + the `--bump` plan/executor
— the executor is now known broken, see above),_ 0.4.0 (#17, pi.dev skill dual-home), 0.3.0 (#16r2, BREAKING —
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
_**Caveat learned this round:** structural safety covers what the engine *does*, not what it
*reports*. Three fail-closed stops worked perfectly in 0.6.0/0.6.1 — but the same engine had been
reporting a green Homebrew leg for six releases while shipping an uninstallable formula. Trust the
refusals; do not trust the green._

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

## Scheduling

Canonical scheduling lives in `issuectl` frontmatter (`lane:`, `lane_seq:`, `blocked_by:`, `collision:`). Do not maintain a markdown DAG or adjacent backlog in this file.

Use these views instead:

```bash
issuectl dag
issuectl dag --json
issuectl ls --status open
issuectl ls --status in-progress
```

`TODO.md` is only the session handoff and project notes; issue bodies and `issuectl dag` are the source of truth.

## Backlog

Post-release hardening + Track B are children/followups under
[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) (still OPEN). `issuectl list` for the
live view. 0.6.1 is shipped; the epic stays open for its tails (see handoff) and the lanes above.

## Piialiisan bugiraportit

- Intake bugs reviewed 2026-08-16. `intake-bug-ossctl-878b3a0790a5` closed fixed because current `release plan` supports `--json`; `intake-feature-ossctl-04e19af4e11d` closed duplicate into `oss-dist-channel-generator`.
- Intake reviewed 2026-08-17: no new items. `intake-feature-ossctl-73e870268475` (release plan/cut
  should cover the Homebrew tap leg) was admitted and is now normal planned work in lane
  `release-safety` — its root cause is a tap declared in the distribution block but absent from
  `targets:`, so it is planned as a green cut that silently skips the leg.
- [x] 🐛 ossctl --version should alias the version verb — **admitted 2026-08-17 (stint #22
  handoff, maintainer ack), now head of lane `cli-canon`**
  ([`intake-bug-ossctl-d9b2ec7bb6d9`](issues/intake-bug-ossctl-d9b2ec7bb6d9/item.md))
