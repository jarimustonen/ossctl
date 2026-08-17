# 0002 — ossctl release engine: adapter model, phase-barrier coordinator, sealed-plan approval seam

**Status:** Accepted
**Date:** 2026-07-25
**Authors:** Maintainer (decision owner); founding-architecture worktree (agent). Pressure-tested via `/llm-panel` (architect=gemini-3.1-pro, maintainability=gpt-5.6-sol, AI-first-CLI-ergonomics=deepseek-v4-pro, release-engineering=claude-opus-4-7).

> Companion to **ADR-0001** (founding spine) and **ADR-0003** (config + journal storage). This ADR settles the most program-shaped part of `ossctl`: how the 6 per-ecosystem release adapters plug in, how the coordinator enforces the irreversibility-ordering rule, and how the approval seam lets an AI-first (non-interactive) binary gate a partially-irreversible publish. The journal *format/location* lives in ADR-0003; this ADR defines what the release engine *does* and what it *records*.

---

## Context

`/oss-release-cut` (design §5, §6.4) is the family's one **stateful, partially-irreversible** operation. Six ecosystems each have a pinned adapter: **rust** (`cargo-publish` / `cargo-dist`), **node** (`release-please` / `changesets`), **python** (`gh-action-pypi-publish` / `twine`), **go** (`goreleaser`), **homebrew** (`homebrew-tap` / `homebrew-core`), **binary** (`gh-releases` / `manual`). A single cut may drive **several** adapters at once (a uv-style `[rust, python]` repo publishes to crates.io *and* PyPI in one cut).

The **irreversibility-ordering rule** (design §5, panel-unanimous): **dry-run ALL targets → build ALL → publish ALL → THEN create+push ONE shared git tag + GitHub Release.** Never tag before every publish succeeds (a pushed tag on a half-published release — crates.io has `0.4.0` that `pip` can't install — is the worst state). Publish is per-target irreversible; there is **no automatic rollback**: the journal records what landed and a human recovers.

Two facts force real structure here, both surfaced sharply by the release-engineering lens:

1. **The approval seam.** AI-first CLIs cannot prompt (§3). The binary must therefore stop at the human-confirm boundary and be re-invoked to execute. Naively ("binary plans, exits; caller re-invokes `cut`") this is **unsafe**: repo state can drift between plan and execute (a new commit, a `Cargo.lock` bump), or the caller can re-invoke with different flags — and the human approved a *different* release than the one that runs.
2. **The 4 phases are asymmetric in reversibility** and each adapter produces **receipts** (published versions, digests, remote URLs) that must be captured as *facts*, not re-derived. Resume behavior differs categorically by which phase's point-of-no-return was crossed.

`octl-core` already proves the durable pattern we build on: an event-sourced journal with append-then-apply atomicity and an idempotent reducer (ADR-0003).

---

## Decision

### 1. `ReleaseAdapter` trait + enum-backed registry; runtime dispatch; all compiled in

Adapter **identity** is the serde enum from `OSS-RELEASE.md` (`cargo-publish`, `cargo-dist`, `release-please`, `changesets`, `gh-action-pypi-publish`, `twine`, `goreleaser`, `homebrew-tap`, `homebrew-core`, `npm-publish`, `manual`). Adapter **behavior** lives behind a trait, one module per ecosystem under `ossctl-core/src/release/adapters/`. All adapters are **compiled in**; selection is **runtime dispatch** driven by the resolved config.

The registry is an **exhaustive enum-backed map, not an unconstrained `Vec<&dyn>`**: composition resolves exactly one implementation per configured target **at startup** and fails fast there if a target has no impl — never mid-release. Compiler-enforced exhaustiveness over the adapter enum guarantees every variant is wired.

```rust
/// Every method takes an injected effect context (CommandRunner, Clock, RegistryQuery — ADR-0001
/// ports) so adapters are unit-testable with a recording fake and never touch the real network.
/// Adapters receive ONLY their own target's slice of the normalized contract — never the whole
/// OSS-RELEASE.md payload — so no adapter can couple to another ecosystem's config (data hiding).
trait ReleaseAdapter {
    fn dry_run(&self, ctx, target) -> Result<DryRunReport>;          // re-runnable, no side effects
    fn build(&self, ctx, target)   -> Result<BuildArtifacts>;        // re-runnable
    fn publish(&self, ctx, target) -> Result<PublishReceipt>;        // PER-TARGET IRREVERSIBLE
    fn verify(&self, ctx, receipt) -> Result<VerifyOutcome>;         // read-only remote reconcile
    fn timeout(&self) -> Duration;                                   // mandatory; hung publish must not wedge a run
}

struct PublishReceipt { adapter, canonical_ref, digest, remote_url, timestamp } // journaled as a FACT
enum  VerifyOutcome  { Matches, Conflicts, Missing, Unknown }                    // drives the resume state table
```

- **No `tag()` method.** Git tagging and the GitHub Release are **structurally owned by the coordinator** — an adapter can never create the shared tag, which is what enforces "tag once, after all publishes."
- **`publish` returns a structured `PublishReceipt`, not `()`** — the canonical ref/digest/URL are captured facts, not values re-derived later (a publish that landed under a drifted version must be detectable).
- **`verify` is required** and returns the typed `VerifyOutcome`. Homebrew/binary adapters with degenerate remote-verify semantics return `Unknown` explicitly rather than being excused from the contract.
- **Mandatory per-adapter timeout + cancellation.** Cancellation is journaled as a **distinct event class from failure** — a cancelled `cargo publish` may have landed remotely and requires `verify` on resume.
- **`ossctl-core` ships an adapter conformance harness**: every adapter is tested for idempotency (given input X and a journaled prior outcome, re-execution yields the same success/conflict/skip verdict) and for the receipt/verify contract, against recording fakes.

### 2. The coordinator: typed phase barriers + partial-failure journaling

A single **coordinator** owns the ordering rule as **typed phase transitions**, not free-form control flow:

```
Phase<DryRunAll> --(all adapters Ok)--> Phase<BuildAll> --(all Ok)--> Phase<PublishAll> --(all Ok)--> Phase<Tag>
```

Each transition is gated on **all targets succeeding** the prior phase (a `TryFrom`-style barrier); a failure in phase *K* blocks entry to *K+1* and journals a specific `phase_completed { phase, outcome: blocked }` event. The **Tag** phase itself splits into three independently-resumable journaled events: `tag_created_local` → `tag_pushed_remote` → `github_release_created`. Phase entry/exit are **first-class journaled events** (`phase_entered`/`phase_completed`) — the linchpin the panel flagged as unstated in the naive proposal.

**No auto-rollback anywhere.** On partial failure the coordinator journals precisely what landed and reports the exact partial state + recovery options (design §5). Any "cleanup on failure" code path is **prohibited** — only journaling. Recovery is the human's, surfaced through `release show` / `release verify`.

### 3. The sealed-plan approval seam (safe non-interactive gating)

The approval boundary is made **machine-checkable** with a content-addressed plan, resolving the unsafe naive seam:

1. **`ossctl release plan`** computes the full intended release — version bump (design §3.4 truth table; for `conventional_commits:false` the skill supplies the human's chosen bump as validated input), per-target commands, target set, ordering — and **seals** it as a content-addressed artifact:
   `plan_id = hash( normalized_contract_json, contract schema_version, git_HEAD_sha, resolved adapter identities+versions, target_set, chosen_version )`.
   `plan` is **read-only** (mutates no external state) and emits a §11 planning envelope + the `plan_id` and a rendered approval instruction. This is the artifact the human approves.
2. **`ossctl release cut --plan <plan_id>`** executes. It **re-hashes current repo state and refuses (`plan_stale` error envelope) if it no longer matches `plan_id`** — so a commit, a `Cargo.lock` bump, a schema-version change, or a different flag set between approval and execution aborts rather than silently publishing something else. There is no execute path without an explicit `--plan`.
3. The binary **cannot cross into the Publish phase** without a `--plan <plan_id>` whose sealed dry-run/build phases are intact. The binary never prompts (§3); the human confirmation happens *outside* the binary, and the chosen SemVer bump is **journaled as an input to the plan**, never passed as loose context held only by the skill.

`release cut` still honors §11 `--dry-run` as a **preview** (re-emit the plan's intended actions as a planning envelope, no mutation) — distinct from `release plan`, which *seals*. `release cut` (execute) is a §12 streaming command (`--output=jsonl`, seq'd events, paired with `release show <run_id>`); a single `--json` document is forbidden for it (§12).

### 4. The full release verb surface (why it grew past `cut`/`show`/`list`)

`plan`, `cut`, `resume`, `verify`, `show`, `list`, `abandon` (ADR-0001 §1). The safety states are **first-class verbs**, not flags, because both the release-engineering lens (safety) and the ergonomics lens (guessability) required them explicit: an agent scanning `--help` for "resume a release" or "what actually landed" must find `release resume` / `release verify`, and un-resumable runs need an explicit terminal `release abandon <id> --reason` so `release list` does not accumulate zombie runs and `resume` never has to guess.

---

## Consequences

**Positive**

- The ordering rule is enforced by **types**, not discipline: a publish cannot precede an all-targets build; a tag cannot precede an all-targets publish; an adapter cannot tag. The worst state (tag on a half-published release) is structurally unreachable.
- The approval seam is **safe under drift**: the human approves a specific sealed `plan_id`, and execution refuses if reality moved. This is the property the naive "plan, exit, re-invoke `cut`" seam lacked.
- Adapters are **isolated and testable**: injected effect ports + per-target config slice + a core conformance harness mean an adapter is unit-tested against a recording fake with no real publish, and a 7th ecosystem is a new module + enum variant + conformance-test row (the compiler enforces wiring).
- Publish **receipts and per-phase events are durable facts**, so `resume`/`verify` reason over what the registries actually hold, not over re-derived guesses.

**Costs / risks accepted**

- **More release verbs than a strict-minimalist surface** (`plan`/`verify`/`abandon` beyond `cut`/`show`/`list`). Accepted: the safety and guessability lenses decisively outweighed §7 minimalism here; each verb is a distinct, first-class run state with a written §7 justification (ADR-0001). (Recorded as a genuine trade-off.)
- **The `verify` primitive is mandatory on all adapters**, including those (homebrew/binary) with weak remote-observability. Accepted via an explicit `Unknown` outcome rather than an optional supertrait — a uniform contract the reconciler can depend on beats a ragged one.
- **Registry-state reconciliation is a genuinely hard correctness surface** (a `Conflicts` from a digest mismatch vs a transient `Missing`). Its resolution is specified as a **documented state table**, not prose — see ADR-0003.

**Rejected alternatives**

- **Feature-flagged adapter compilation** (compile only the needed adapters). Rejected: a cut's target set is a *runtime* property of `OSS-RELEASE.md` (uv repos = rust+python) — you cannot know the combination at compile time, so feature flags would break multi-ecosystem cuts; and adapters are thin subprocess wrappers, so there is ~zero binary-size or dependency win to gate.
- **Unconstrained `Vec<&dyn ReleaseAdapter>` registry.** Rejected in favor of an enum-backed exhaustive registry: the latter fails an unwired/missing adapter at **startup** with compiler-enforced coverage, not mid-release.
- **A `tag()` method on the adapter trait.** Rejected: it would let a per-ecosystem adapter create the shared tag and defeat "tag once, after all publishes." Tagging is coordinator-only.
- **`publish` returning `Result<()>`.** Rejected: the canonical ref/digest/URL must be journaled facts to detect a publish that landed under a drifted version and to drive `verify`.
- **The naive approval seam** ("binary plans, exits; caller re-invokes `cut`" with no binding). Rejected as unsafe: nothing prevents repo drift or a mismatched re-invocation between approval and execution. Replaced by the content-addressed, drift-checked `plan_id`.
- **Automatic rollback of a partial failure.** Rejected per the locked design: irreversible steps (a published crate, a pushed tag) cannot be truly rolled back; a fake rollback is more dangerous than an honest partial-state report. The engine only journals and surfaces recovery options.
- **Folding `plan`/`resume`/`status` into flags on `cut`.** Rejected: `--dry-run` cannot express both "seal an approval artifact" and "the mandatory dry-run-all phase of a real cut", and hidden `--resume`/`--verify` flags fail the guessability bar for distinct safety states.

---

## Addendum (2026-08-05) — GitHub Release ownership when a CI-delegated target is present

The tag phase above states the coordinator creates the shared git tag **and** the GitHub Release. That is correct for a plan with no CI-delegated target. But `ossctl`'s own contract (and any contract that ships a `cargo-dist` / `gh-releases` target) has a tag-triggered CI workflow that ALSO creates the Release — so an engine-driven cut would create the Release from the tag, then cargo-dist's `release.yml` would fire on the same tag and clash. Decision (`coordinator-release-vs-cargo-dist-ownership`, maintainer, **Option 1 — CI owns it**):

- When the plan carries **≥1 target whose CI owns the GitHub Release**, the coordinator **creates and pushes the tag but does NOT create the GitHub Release** — the pushed tag is what triggers CI, and CI owns Release creation + the cross-platform binary upload. The delegation is journalled as `github_release_delegated` (carrying the owning adapter, in place of `github_release_created`), so resume/verify treat the missing engine-created Release as intentional, never a step to re-attempt.
- With **no** such target, the coordinator still creates the Release itself — unchanged ADR-0002 behavior.
- **The predicate is a NARROW `ci_owns_github_release()`, a strict subset of `is_ci_delegated()` — not the broad delegation flag.** `is_ci_delegated()` answers "who runs this target's *publish*"; it is true for three adapters (`cargo-dist`, `release-please`, `gh-action-pypi-publish`). But only `cargo-dist` runs `gh release create` for the tag. `gh-action-pypi-publish` uploads to **PyPI** and `release-please` is publish-on-merge — neither touches the GitHub Release, so reusing `is_ci_delegated()` here would wrongly suppress the engine-created Release for a pure-Python trusted-publisher plan (surfaced in review). Hence the distinct `ci_owns_github_release()` capability, overridden `true` only by `cargo-dist`. A contradictory already-recorded disposition (a delegation demanded over an engine-created Release, or the reverse — only reachable if a resumed run's binary reclassifies the adapter) is refused as a tag-phase failure, never merged into a dual-disposition state.

**Why Option 1 and not Option 2 (upload into an existing Release).** cargo-dist's generated `release.yml` `host` job finalizes the Release with `gh release create "<tag>" --target <sha> --title … --notes-file … artifacts/*` (verifiable in this repo's `.github/workflows/release.yml`). `gh release create` **fails if a Release for the tag already exists** — it is a create, not an upsert/edit. So a coordinator-pre-created Release does not merely duplicate work; it would make cargo-dist's `host` job **error out** ("release already exists"), leaving the cross-platform binaries unpublished. Option 1 (tag-only when CI-delegated) is therefore not just cleaner but **required** for this flow: cargo-dist must find no pre-existing Release for the tag, then it creates and fully finalizes it. This is confirmed from the repo config alone; the first engine-driven `0.2.0` cut is still the live end-to-end check.

---

## Amendment (2026-08-06) — cargo-ecosystem interleave (deferred packaging for a `=`-pinned dependent)

The Decision above states the phase barrier as **build ALL → publish ALL**: every target is packaged in build-all before any target is published. The first engine-driven cut of `ossctl` itself (0.2.0, runs `01KZB29W…` then `01KZB40Y…`) proved that barrier is **incompatible with cargo's multi-crate `=`-pinned publish model**, and this amendment scopes one narrow exception. ADR-0004 (one target = one publish unit) is **unchanged**.

**The problem.** `ossctl`'s workspace has two crates.io targets — `ossctl-core`, then the dependent `ossctl`, which pins `ossctl-core = "=X.Y.Z"` (the exact shape `/oss-init` emits). `cargo package -p ossctl` (the build-all step) resolves that `=`-pinned dependency **against the crates.io index** while preparing the upload — a published `.crate` cannot reference a `path` dep, so cargo must find the exact version on the index. But `ossctl-core X.Y.Z` is only *published* later, in publish-all. So the dependent **cannot be packaged in build-all**, and `--no-verify` does not help: it skips only the isolated verify *compile*, not this index resolution (confirmed on the re-cut; the earlier `--no-verify` build-phase hack, bfb05d3, was insufficient). A strict build-ALL therefore cannot cut any multi-crate workspace with `=`-pinned internal deps. (Filed and diagnosed in `release-cut-build-phase-dep-ordering`.)

**The decision.** For **same-ecosystem, dependency-ordered cargo (`cargo-publish`) targets**, the dependent's *packaging* **interleaves with publish** in dependency order — `publish ossctl-core → wait for the crates.io index → package + publish ossctl` — instead of packaging up front in a global build-all. Concretely:

- The **cargo adapter** classifies each target by reading the workspace graph (read-only `cargo metadata`) **and probing the registry**: does the target depend on a publishable workspace crate whose exact version is **not yet on the crates.io index**? A target with **no such dependency** — a leaf, *or* a dependent whose workspace deps are already published (a re-cut) — is packaged in build-all exactly as before (`cargo check` then `cargo package --no-verify`, producing the `.crate` and validating the manifest): it *can* be packaged, because `cargo package` resolves the dep against the index it is already on. A **dependent on a not-yet-published workspace crate** runs only the **index-independent `cargo check`** in dry-run/build (the sibling resolves via its on-disk `path`, never the index), and its **packaging is deferred to `cargo publish`** in publish-all — which packages **and** publishes as one unit, *after* the dependency is published and index-visible. The probe is **fail-closed**: a dep the registry cannot confirm as published defers, so an outage never risks a build-all `cargo package` against an unreachable index. This is the *precise* predicate — "defer iff a workspace dep is not yet on the index" — not the coarser "has any workspace dep", so the manifest-validation safety net is preserved whenever packaging is actually possible.
- The **coordinator is not special-cased.** publish-all already walks same-ecosystem targets in dependency order (`release-cut-multi-target-ecosystem`), and the adapter's `publish` already index-waits on the target's own workspace deps (ADR-0004). So `publish core → wait index → package+publish cli` falls out of the *existing* dep-ordered publish phase; the only change is that the dependent's package step is intrinsic to its publish rather than a premature global-build step. This keeps rust-specific workspace reasoning inside the cargo adapter (ADR-0002 §1 data-hiding) rather than pushing it up into the coordinator.

**Why this is a scoped exception, not a barrier rewrite.** The outer barrier is preserved:

- **`dry-run-all` still runs first** across every target (for cargo, the index-independent `cargo check` + a leaf's `--no-verify` package), so a plan that cannot compile fails at dry-run — before any external effect. This is also the "safer dry-run" the reopen note asked to keep: dry-run *runs* the compile gate rather than only describing a `cargo publish --dry-run`. (The dry-run no longer runs `cargo package` for a dependent — that step could never pass pre-publish — so the false-fail at dry-run is removed while the real compile preflight stays.)
- The **pre-publish compile safety net is still a global build-all barrier.** Every target's `cargo check` runs before **any** irreversible publish, so a compile error in the **default host build** (type/trait/API mismatch, missing item) fails the cut before anything lands — the partial-publish trap ADR-0004 exists to prevent. (`cargo check` is not full package verification — it does not exercise the packaged file set, non-default features, or all targets — but it is the strongest index-independent gate available before the dependency publishes.) Only the dependent's *packaging* (which cargo fuses with publishing anyway) moves into the publish phase.
- **Tagging is still coordinator-only and once-after-all-publishes**, and the **post-tag homebrew (dist) phase is unchanged.** Only *within* the group of same-ecosystem dep-ordered cargo/crates.io targets does packaging interleave with publish; every cross-ecosystem and CI-delegated ordering is exactly as the Decision states.

**Resumability (ADR-0003, remote-is-ground-truth).** The interleave adds no new journal event and does not change the per-target skip logic, so `resume`/`reconcile` are unaffected. A cut that dies after publishing `ossctl-core` but before `ossctl` resumes correctly: publish-all skips the already-recorded `ossctl-core` (never re-publishing an irreversible crate) and completes `ossctl` (whose deferred packaging now succeeds, its dependency being on the index). Covered by `resume_after_core_publish_completes_the_dependent_without_republishing_core` (coordinator) and the adapter-level `target_skips_its_own_publish_when_already_published_on_resume`.

**Residual risk (documented, and now narrowed).** For a dependent whose workspace dep is **genuinely not yet on the index** (the normal lockstep cut), its *manifest/packaging* validity (bad `license`, an excluded required file) can only be checked by `cargo package`/`cargo publish`, which cannot run until the dependency is on the index — so such an error surfaces at the dependent's `cargo publish`, **after** its dependency has published (a torn release the engine journals but cannot roll back). This window is **inherent to cargo's model** for that case — the manual fallback recipe carries the identical risk — and it is bounded by the index-independent `cargo check` (a default-host compile check, not full package verification). The registry-aware predicate **removes the avoidable part**: a dependent whose deps are already indexed is *not* deferred, so it keeps its full build-all `cargo package` manifest validation. Two further narrowings are follow-ups, not owned here: (a) a plan-time coverage/packaging preflight (e.g. a build-time `cargo package` against a `path`-rewritten manifest, if that can be made not to resolve the `=`-pin against the index — unverified) to catch a genuinely-deferred dependent's manifest errors before its dependency publishes; and (b) rejecting a workspace path-dependency with no publishable version requirement up front rather than at `cargo publish`.

---

## Amendment (2026-08-17) — mandatory post-cut verification: green means observed

The coordinator runs a final `verify` barrier after `dist`. A v5 journal reaches
`Completed` only after every published receipt and CI-delegated target has an
observed-good result at its destination. A v1–v4 journal remains compatible: its
successful `dist` event is terminal.

A publish target that cannot be observed after the fact is not a publish target.
Registry receipts are re-checked through the registry seam; Homebrew formulas are
fetched from the tap and checked for the ossctl marker, sealed version, and platform
stanzas; cargo-dist GitHub Releases are polled through the command seam for their
expected archives. `Unknown` is not green: it journals an honest observation but
fails the barrier alongside `Missing` and `Conflicts`. `--allow-unverified` remains
resume-only and never makes an unobserved fresh cut complete.
