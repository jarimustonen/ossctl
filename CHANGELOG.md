# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- oss-changelog:unreleased-start -->
## [Unreleased]

## [0.10.0] - 2026-08-20

### Added
- `ossctl facts --json` now exposes `data.cargo_publish`, the per-manifest
  `allowed` / `forbidden` / `unknown` evidence (with workspace inheritance resolved)
  used by the contract normalizer's crates.io publish floor. Floor errors point to
  this command so operators can inspect exactly what ossctl read.

### Fixed
- **Release plans and cuts now agree on repeated exact workspace pins.** A crate that
  declares the same internal `=version` dependency in normal, dev, build, or
  target-specific dependency tables seals one deterministic rewrite set and updates
  every equivalent declaration during the bump. Mixed requirements for the same
  workspace crate are refused while planning, before an unexecutable approval artifact
  can be sealed. This changes the meaning of a sealed pin rewrite, so release-plan
  `SEAL_VERSION` advances from 6 to 7: plans sealed by an older binary still load for
  resume, but cannot start a fresh cut and must be re-planned and approved
  (`intake-bug-ossctl-d38ddf598fd5`).
- **Sealed plans can now be abandoned before a run starts.** `release abandon <plan_id>`
  removes the authenticated plan document without creating a journal, remains idempotent on
  retry, and redirects to the existing run-abandon path when that plan already backs a run
  (`abandon-sealed-plan-id`).
- **Release cuts no longer double-write cargo-dist-owned Homebrew taps or hide what landed after a post-tag failure.** A cargo-dist `publish-jobs = ["homebrew"]` configuration now refuses an engine-owned `homebrew-tap` target in favor of the delegated `cargo-dist` target, while ossctl's own engine-owned tap path remains intact. A transient tap-clone failure retries with bounded backoff (later, ambiguously mutating steps never retry blindly), and an ordinary terminal dist-phase failure still observes every destination without marking the failed run complete (`cut-runs-own`).
- **GitHub Releases produced by cargo-dist are now verified at their actual destination.**
  Release verification resolves the published `v<version>` tag, ignores cargo-dist's
  human-formatted Release title, and observes cargo-dist's own manifest instead of
  inventing archive names from the contract's broader install-platform policy. This
  removes false `missing` results for complete Releases whose project/package naming or
  cargo-dist target set differs from that policy (`verify-gh-release-missing`).

## [0.9.0] - 2026-08-17

### Added
- **CLI help is now machine-readable.** `ossctl --help --json` and nested command help
  emit the canonical success envelope with clap-derived subcommands, flags, positional
  arguments, defaults, accepted values, environment mappings, and structured examples
  (`cli-canon-help-json`). Text help remains unchanged.
- **A repo that publishes to crates.io from CI can now use `release cut`.** A target
  declared as `registry: crates.io, adapter: cargo-publish-ci` is CI-delegated: the cut
  runs the local gates, bumps, and pushes the tag — which is what triggers the repo's
  publish workflow — but never runs `cargo publish` from the host, so it can neither
  double-publish against the workflow nor fail on a stale local token. The mandatory
  verify barrier then polls the registry index until CI's publish is observed, with the
  same bounded wait used for delegated GitHub Releases; an unobserved target still fails
  the cut, and a registry outage reports `Unknown` rather than a false `Missing`.
  Delegation is per target, so a contract may mix engine-published and CI-published
  targets (`release-ci-publish-mode`).

  Three refusals keep the mode honest, all of them before the irreversible tag push:
  a delegated target whose version is **already** on the registry is rejected in
  dry-run (post-tag, verify would otherwise observe the pre-existing upload and go
  green over a CI publish that failed); a package declared with both publishers is a
  contract floor; and an engine-published crate that depends on a CI-delegated crate
  in the same workspace is refused by `release cut`, because publish-all runs before
  the tag that triggers CI and that wait can never be satisfied.
- **A repo that publishes NOTHING can now be released.** Publish-none (an authored
  `targets: []` — a private service deployed by its own script, `publish = false` in
  its manifest) is a first-class contract shape, distinct from CI-delegation (where
  something *is* published, just not by the engine). Its version is derived from the
  tree's own manifests rather than through a publish target, so `release plan` no
  longer refuses with `version_undeterminable`; the cut is **tag-only** — nothing is
  published, no GitHub Release is created or delegated, and the verify barrier passes
  vacuously because there is genuinely nothing to observe (still categorically
  distinct from `Unknown`, which stays a barrier failure). `release cut`'s summary says
  "published nothing — tag-only cut" rather than reporting a count
  (`publish-none-unrepresentable`).
- **The contract now cross-reads `Cargo.toml`'s `publish` key.** A declared crates.io
  target for a crate whose manifest sets `publish = false` (or `publish = []`, or an
  allow-list without `crates-io`) is a normalization floor — the publish could never
  succeed, and for a CI-delegated target the first symptom would otherwise be a red
  workflow after the irreversible tag. Conversely, a publish-none contract whose
  manifests explicitly allow publishing normalizes cleanly with a warning naming them:
  the contract holds, but nothing in the tree stops an accidental `cargo publish`. The
  read covers the root package **and** every workspace member (including a hybrid
  `[package]` + `[workspace]` root) and resolves `publish.workspace = true`
  inheritance. It is a tri-state: a manifest whose `publish` cannot be resolved yields
  no diagnostic in either direction, as does an unreadable manifest or a target naming
  a package no manifest declares — absence of evidence is never evidence.
- **A binary `distribution` block alongside an empty `targets` is now refused.** The
  two contradict each other: the engine would cut a tag-only release while the pushed
  tag triggers cargo-dist/goreleaser to publish binaries the run never planned or
  verified.

### Fixed
- **Closed stdout pipes no longer panic the CLI.** Text and JSON output now share a
  fallible writer: a downstream reader closing early exits quietly with success, while
  other stdout failures use the canonical system-error envelope (`cli-panics-broken`).

## [0.8.0] - 2026-08-17

### Fixed
- **Sealed release plans now disclose the mandatory verification barrier.** `release plan` includes `verify` after `dist`, matching the coordinator's real cut sequence; the phase list is part of the approval seal, so its seal format advances to v6 and previously stored v5 plans require a fresh approval for new cuts.
- **Single-crate release bumps now update the root package manifest.** `release --bump`
  falls back from `[workspace.package] version` to `[package] version` only when the
  workspace source is absent, while retaining lockfile refresh, CHANGELOG finalization,
  hook validation, and fail-closed version checks (`bump-single-crate-manifest`).
- **`ossctl --version` and `ossctl -V` now alias `ossctl version`.** The aliases preserve the
  selected output format, including the canonical JSON envelope (`version-flag-alias`).
- **CI-owned Homebrew taps are now declared and verified without a double writer.** A
  `registry: homebrew, adapter: cargo-dist` target is normalized as a real release
  surface, skips the engine tap-write in favor of cargo-dist's CI job, and verifies
  the published formula version without requiring ossctl's engine-owned marker
  (`homebrew-ci-delegated-adapter`).

## [0.7.0] - 2026-08-17

### Fixed
- **CLI error exits now distinguish caller-actionable failures from operational faults.**
  Missing `OSS-RELEASE.md` contracts now exit `1` rather than `2`; agents branching on
  ossctl's exit status can reliably treat `1` as an input/current-state problem and `2` as
  a retry-or-escalate fault (`cli-canon-exit-codes`).
- **Release completion now means every publish target was observed at its destination.** A new mandatory `verify` barrier checks registry receipts, Homebrew tap formulas, and GitHub Release assets after `dist`; missing, conflicting, and unobservable targets leave a resumable failed run instead of a false-green cut.
- **`release abandon` now recovers a hard-killed release run's stale lock.** Lock files record
  their PID, hostname, and start time. `abandon` removes one only when the same-host holder is
  provably gone (`kill -0` reports `ESRCH`), then retries once and records what it did. Live,
  malformed/legacy, unidentifiable, and other-host locks still fail with an actionable
  `cut_in_progress` error (`release-abandon-break-stale-lock`).
- **Release plans are now durable approvals.** `release plan` saves its sealed plan under the git common directory; `cut` recovers an omitted `--bump` from it, and `resume` can continue a failed run after a code fix moved HEAD. The `plan_stale` error now reports `recomputed_plan_id` rather than `current_plan_id` and never recommends a replacement plan id.
- **Release cuts now refuse under-declared distribution surfaces before creating a run.** Facts
  detect cargo-dist configuration and tag-triggered workflows, validation and planning warn when
  their GitHub Release target is missing, and `release cut` returns
  `undeclared_distribution` until the contract declares the delegated GitHub Release and any
  unserved Homebrew tap target.

### Internal
- Add a hermetic end-to-end release harness that drives the compiled `ossctl` binary through planning, failed cuts, journal inspection, abandonment, and lock refusal without invoking real publish tooling.
<!-- oss-changelog:unreleased-end -->

## [0.6.1] - 2026-08-17

### Fixed
- **The Homebrew leg no longer fails on platforms Homebrew cannot serve.** 0.6.0's prebuilt-archive
  renderer treated every entry in `distribution.platforms` as one it had to serve and errored on the
  rest — so a contract that legitimately declares a Windows binary (built by cargo-dist and attached
  to the GitHub Release, which Homebrew simply does not serve) could not complete its dist phase.
  This blocked ossctl's own 0.6.0 cut after crates.io and the tag had already landed. The renderer
  now selects the servable platforms (macOS aarch64/x86_64, Linux musl aarch64/x86_64) and ignores
  the rest. A Homebrew target with *no* servable platform is still an error, and a servable platform
  whose asset is missing still fails closed (`homebrew-skip-unsupported-platforms`).

### Changed
- **Windows is no longer built or shipped for ossctl itself** (maintainer decision): the
  `x86_64-pc-windows-msvc` target and the PowerShell installer are removed from this repository's
  contract and cargo-dist configuration. Building a Windows binary cost CI time on every release for
  a platform that is not used. Note this changes only *this repository's* opt-in — Windows has never
  been in the normalizer's default platform set, so no other project is affected and no generated
  default changes.

## [0.6.0] - 2026-08-17

### Fixed
- **The generated Homebrew formula could not be installed at all.** Every formula the engine wrote
  since 0.2.3 ran `cargo install` against the extracted tarball root, which fails on a virtual
  workspace manifest (`found a virtual manifest ... instead of a package manifest`). Six releases
  reported their Homebrew leg green while publishing an uninstallable formula. The formula now
  renders per-platform archive URLs with their checksums, installs the prebuilt binary, carries the
  real package description, and smoke-tests a command that exists (`homebrew-formula-uninstallable`).
- **The Homebrew leg no longer builds from source.** The post-tag dist phase waits (bounded, 300s)
  for the cargo-dist release assets, fetches and hashes them, and renders the formula against the
  published binaries. It **fails closed** if the assets do not appear — there is no source-build
  fallback and no placeholder checksum, because a tap silently left on an older version is the
  defect this replaces. Homebrew is the family's primary install channel and no longer requires a
  Rust toolchain.
- **The stale-binary guard no longer blocks downstream releases.** As shipped earlier the guard
  compared the binary's compiled commit against the *release tree's* HEAD, which a downstream
  repository can never satisfy — so `release plan`/`cut` refused everywhere except this repository,
  and `--allow-stale-binary` became a reflex that would have hollowed out the guard for the self-cut
  too. The check is now scoped to canonical self-cuts (`stale-binary-guard-downstream`).
- **Preserved contract fields no longer drop values silently.** `yaml_to_json` coerced non-string
  mapping keys inside preserved content, so `42:` and `"42":` in one mapping collapsed and one value
  was lost with no warning. Non-string keys now produce a path-carrying error. The canonical JSON
  shape for valid contracts is unchanged, so `schema_version` is unchanged
  (`extra-fields-nested-nonstring-yaml`).
- **Flaky macOS CI.** Test fixture workspaces were named from the process id plus a clock reading, so
  sibling tests sharing a process could collide and overwrite each other's manifests. Fixtures are now
  uniquely allocated and cleaned up on drop (`temp-workspace-dir-collision`).

### Added
- **`ossctl config path` and `ossctl config show --json`.** Report where the effective configuration
  lives and what each value resolves to, with its provenance (default, file, environment, flag), so an
  agent can answer "why is this value what it is" without scraping prose (`cli-canon-config`, canon §8).
- **`/oss-dist` — the distribution generator.** The `/oss-*` family had no member for the
  `gh-releases`/`cargo-dist` and `homebrew` channels, so a contract declaring a binary or Homebrew
  channel had to be hand-wired per repository. The new member emits the cargo-dist configuration,
  wraps `ossctl dist generate`, ensures the dist profile, and documents the tap and token setup, with
  macOS arm64/x86_64 and Linux musl arm64/x86_64 coverage enforced (`oss-dist-channel-generator`).

### Changed
- **`contract validate` warns when a Homebrew tap is declared only in `dist-workspace.toml`.** The
  contract remains the single source of truth, so a tap the contract does not carry is not planned —
  previously in silence. Absent or unparseable cargo-dist configuration stays silent
  (`contract-validate-warn`).
- **A contract that declares `distribution.homebrew_tap` with nothing to produce the formula is now
  rejected** rather than planned as a green cut that skips the tap. Deriving the target implicitly was
  considered and rejected: a distribution block cannot safely identify which package of a multi-crate
  workspace the formula is built from.

## [0.5.0] - 2026-08-13

### Added
- **Engine-owned version bump: `ossctl release plan --bump major|minor|patch`.** The release engine
  now computes the new version from the workspace manifest plus a semantic *level* (the human supplies
  only major/minor/patch — no hand-typed literal, honoring the 0.3.0 single-source-version decision)
  and seals it as a content-addressed bump phase. `ossctl release cut` **executes** that phase inside
  the clean checkout of the sealed commit: it sets `[workspace.package] version`, applies precise
  intra-workspace `=<version>` pin rewrites, refreshes `Cargo.lock`, finalizes the CHANGELOG, runs any
  contract-declared `release.bump_hook`, commits, and points the release tag at the *bump* commit. The
  bump is resume-safe (a `BumpApplied` journal fact prevents a double-bump on an interrupted cut) and
  the no-bump path is byte-for-byte identical (unchanged `plan_id`). Omitting `--bump` preserves the
  existing behavior — the engine publishes the version already in the tree
  (`release-rust-workspace-multicrate`, facets 2+3).
- **Contract-declared `release.bump_hook`.** An optional contract field naming a project command the
  bump phase runs after applying the version edits (e.g. to regenerate version-embedding snapshots),
  surfaced verbatim with a post-hook version-validation guard. Additive/optional — existing contracts
  serialize unchanged.
- **Dependency-ordered multi-crate workspace publish.** `ossctl release plan` now derives the full
  dependency-ordered publish set for a multi-crate Rust workspace (lib before bin) from a **bin-only**
  contract — the transitive dependency *closure* of the declared targets, with precise path/workspace
  edges — so a downstream two-crate workspace no longer fails its cut on an unpublished `=`-pinned
  library. Only the declared targets' closure is swept in (never unrelated publishable members)
  (`release-rust-workspace-multicrate`, facet 1).

### Changed
- **`homebrew_tap` is carried from the contract's `distribution` block into the sealed plan** (was
  `null`), so a multi-crate workspace's Homebrew leg is planned correctly
  (`release-rust-workspace-multicrate`, facet 4).

## [0.4.0] - 2026-08-11

### Added
- **`ossctl skill install` dual-homes into pi.dev.** A new `pi` runtime writes each bundled
  `SKILL.md` into `~/.pi/agent/skills/<name>/` (discovered by the pi.dev harness as `/skill:<name>`),
  and `--agent` gains `pi` (narrow to pi.dev only) and folds pi.dev into `all`
  (`pidev-dual-home-skills`).

### Changed
- **BEHAVIOR: `ossctl skill install` now dual-homes by default.** With `--agent` omitted the installer
  writes each skill into **both** `~/.claude/skills` (Claude Code) **and** `~/.pi/agent/skills`
  (pi.dev) — previously it wrote Claude only. Pass `--agent claude` to restore the old single-home
  behavior; `--agent pi`/`codex` narrow to one runtime; `--agent all` targets every known runtime. The
  `--json` `installed[]` object shape is unchanged (additive), but the default now emits two rows and
  writes two targets where it emitted/wrote one — automation that assumed Claude-only or a fixed row
  count should pin `--agent claude` (`pidev-dual-home-skills`).

## [0.3.0] - 2026-08-11

### Removed
- **BREAKING (CLI): `--version` is removed from `ossctl release plan` / `release cut`.** The release
  version now derives *solely* from the workspace manifest (`Cargo.toml`) — the single source of
  truth. A stray `--version` is a hard `unexpected argument` error rather than a silently-ignored (or
  drift-guarded) flag. To release a new version, bump the manifest (and finalize the CHANGELOG) in a
  release commit first, then `plan`/`cut` (`release-drop-version-flag`; completes the 0.2.5
  single-source work).

### Added
- **Version-source capability model — non-Rust ecosystems no longer fail open.** The version-drift and
  self-visibility guards now key on a per-ecosystem version-source: manifest-versioned ecosystems
  (rust/node/python) fail **closed** when a target's version can't be read, while distribution-only
  targets (homebrew / raw binary / cargo-dist) are legitimately skipped. Previously any target without
  a readable manifest version was silently skipped, so the guards were no-ops for npm/PyPI packages
  (`version-source-fail-closed-nonrust`).

### Changed
- **`release cut`/`resume` publish from a clean checkout of the sealed commit.** The engine now
  materializes a fresh git-worktree checkout of the sealed plan's `head_sha` and runs build/publish/dist
  from there, instead of the live (mutable) working tree — a cut is reproducible and immune to mid-cut
  edits. It fails closed if the sealed commit isn't available locally; the journal and tag still land in
  the real repository (`release-cut-clean-checkout`).
- **The resume idempotency skip is digest-authenticated.** When a resumed cut finds a crate already
  published, it now repackages the target `.crate`, hashes it, and compares against the registry's
  published checksum (crates.io sparse-index `cksum`) before trusting the skip: a match records the
  digest, a mismatch fails closed (`DigestMismatch`), and an outage/malformed response fails closed
  (`RegistryUnavailable`) — closing the last "receipt without a verified artifact" path. (The definitive
  cross-toolchain-safe form — journaling the intended digest at original-publish time — is tracked in
  `cargo-publish-receipt-provenance-resume-safety`.) (`is-published-digest-authenticate`)

## [0.2.5] - 2026-08-10

### Fixed
- **A `release cut` can no longer report success while publishing nothing.** After the irreversible
  `cargo publish`, the cargo adapter now confirms the target's own `{name, version}` is actually
  visible on the crates.io index (reusing the bounded index-wait, so normal sparse-index propagation
  lag is tolerated) *before* journaling a publish receipt. A silent no-op upload now fails the cut
  closed with no fabricated receipt; a registry outage fails closed distinctly from "reached the
  registry, version absent". This closes the real-world failure mode where a downstream cut reported
  `build ok → publish` yet the crate never reached crates.io (`cut-noop-self-visibility-check`,
  surfaced by the first real downstream cut).

### Changed
- **The release version now has a single source of truth: the workspace manifest.** `ossctl release
  cut` publishes the version already in the tree and derives it from `Cargo.toml`; `--version` is now
  an *optional confirmation* that must equal the manifest version (a mismatch refuses the plan/cut),
  subsuming the earlier drift guard. The documented recipe (`release plan --version X.Y.Z`) keeps
  working unchanged (`release-version-single-source`, `release-cut-publish-noop`).

### CI
- **Generated/own `publish-crates.yml` is now idempotent.** Both `cargo publish` invocations in the
  dep-order step tolerate cargo's exact per-package "already exists on crates.io index" diagnostic as
  success (anchored match; genuine failures still fail), so a successful engine cut no longer produces
  a spurious red CI run when the tag-push publish races the engine's own publish (`publish-crates-yml`).

## [0.2.4] - 2026-08-10

### Added
- **The contract can now express "version-tracked but never published".** An explicitly-set empty
  `targets: []` is honored as authoritative — the normalizer no longer force-expands it into a default
  `crates.io`/`cargo-publish` target. An *omitted* `targets` key still expands to the ecosystem default
  as before (the distinction is explicit-empty vs absent). This lets a private/internal project be
  version-tracked and changelogged without any registry publish target.
- **Multi-distribution (monorepo) support.** `Contract` now carries `distributions: Vec<Distribution>`
  with a per-package `package` association key, so a monorepo can declare several independently
  distributed binaries (each with its own gh-releases/installers/tap). A single bare `distribution:`
  block still parses unchanged (deserialized as a one-element list); a plural `distributions:` list is
  also accepted. The release engine remains single-distribution and fails loud on a `distributions`
  length > 1 (per-distribution engine support tracked in `per-distribution-release`).

### Changed
- **BREAKING (canonical wire shape): `schema_version` bumped `1` → `2`.** The canonical-JSON key
  `distribution` was renamed to `distributions` (now always an array), and every distribution carries a
  `package` field. ossctl still **reads** v1 documents (a singular `distribution:` mapping) and
  translates them into the v2 canonical shape. Downstream `/oss-*` members that read the normalized
  contract should key on `distributions`.
- **The normalizer now hard-errors on inconsistent Homebrew configuration** (previously silent). A
  `homebrew`-registry target or a `homebrew` installer with no `homebrew_tap` destination, a
  double-publish collision (installer **and** target both producing a formula), and a
  registry/adapter mismatch (`registry: homebrew` requires the `homebrew-tap` adapter) are now rejected
  at validate time with a clear error, instead of failing later at release time. This can reject some
  previously-accepted contracts — intended.
- **The engine's Homebrew tap-write preserves hand-maintained formulas.** A formula is fully
  regenerated only when it carries an `ossctl`-generated ownership marker; a hand-maintained formula
  (no marker) is updated surgically (only the `url`/`sha256` lines) or refused, never clobbered. ossctl's
  own generated formula carries the marker, so its own tap keeps the simple full-render path.
- **An empty `extra_fields` map is now omitted from canonical JSON** (not emitted as `"extra_fields": {}`),
  symmetrically for the top-level `Contract` and the nested `Distribution` (`skip_serializing_if`). A
  populated `extra_fields` serializes exactly as before. No `schema_version` change from this alone, but
  the release-plan `SEAL_VERSION` bumped `4` → `5` (the sealed pre-image of an empty-`extra_fields`
  contract changed). Combined with the monorepo change above, `SEAL_VERSION` is `5` and `schema_version`
  is `2` in this release.

### Fixed
- **The generated crates-publish CI workflow now auto-fires.** It triggers on the version-tag `push`
  instead of `release: published` — GitHub does not emit a `release` event for a Release created by the
  default `GITHUB_TOKEN` (as cargo-dist does), so the old trigger never ran and crates.io was published
  only via manual dispatch. `workflow_dispatch` is retained as a manual fallback.
- **`release resume` no longer demands `--allow-unverified` when the publish phase was never reached.**
  A run that failed in the build phase (before any publish could have happened) now resumes directly;
  the genuinely-unsafe cases (publish reached but no receipt; `Published × Unknown`) still require the
  flag.
- **`release abandon --reason` accepts a reason that starts with `--`** (previously rejected as an
  unknown flag by the argument parser).
- **Normalizer diagnostics JSON-encode user-controlled keys**, so a contract key containing quotes,
  newlines, or control characters can no longer forge or corrupt diagnostic/JSONL log lines.

## [0.2.3] - 2026-08-07

### Fixed
- **The engine's Homebrew release leg is now self-sufficient — no local `brew` toolchain required.**
  For a configured tap with an existing formula, the `dist` phase renders `Formula/<name>.rb` from the
  verified sha256 and pushes it **directly to the tap** (git/API write), instead of shelling to
  `brew bump-formula-pr` (which ran `brew audit` internally and could abort a cut on an unrelated
  audit/lint error, as it did on the 0.2.2 cut). The write verifies the sha against the fetched
  archive, guards formula existence/symlinks, and is idempotent (a byte-identical formula is a no-op).
  `brew bump-formula-pr` is retained only for the homebrew-core path. This makes an engine cut's
  Homebrew leg deterministic and hands-off.
- **False-positive `homebrew_tap` contract warning removed.** The normalizer no longer warns
  "homebrew_tap is set but no homebrew installer" when the contract declares a `homebrew`-registry
  **target** — that target IS the tap consumer, so the tap is updated. The warning now fires only when
  a tap has neither a homebrew installer nor a homebrew target (the genuinely-orphaned case).

### Added
- **Forward-compat capture on the nested `distribution` block.** The `Distribution` struct and its
  sub-structs now preserve unknown sub-keys in `extra_fields` (mirroring the top-level `Contract`), so
  an older ossctl reading a newer contract round-trips distribution keys instead of dropping them.
- **Installer↔platform coherence warning.** The normalizer now warns when `distribution.installers`
  targets an OS absent from `distribution.platforms` (e.g. `msi` without a Windows triple, `homebrew`
  without a macOS or Linux triple), catching dead installer config; `npm`/`shell`/`powershell` are
  OS-agnostic and ungated.

## [0.2.2] - 2026-08-06

### Added
- **crates.io `RegistryQuery` for ecosystem `rust`** — the release engine's publish/reconcile
  path can now determine a crate@version's already-published state by querying the crates.io
  **sparse index** (`index.crates.io`, via `curl` with a bounded `--max-time`; no new HTTP
  dependency). Previously only `node` (npm) was wired, so a `rust` cut's publish phase failed
  **closed** with *"no registry query wired for ecosystem 'rust' yet"* — the last blocker
  before the engine could cut ossctl itself. The query is defensively fail-closed: a genuine
  registry-unreachable (network error, unexpected HTTP status, malformed index line) returns an
  error rather than a false "not published"; a `404`/`410` is the legitimate "not yet published"
  signal (empty result); yanked versions count as published (their version slot is occupied).
  This completes the engine's registry-aware defer/idempotency predicate for crates.io — the
  cut of ossctl itself now runs end-to-end through its own engine (dogfood).

## [0.2.1] - 2026-08-06

### Fixed
- **Multi-crate cuts with `=`-pinned internal deps now cut through the engine.** For a workspace
  whose dependent crate pins its workspace dep by exact version (`ossctl` → `ossctl-core = "=X.Y.Z"`),
  `ossctl release cut` no longer fails in the build phase trying to package the dependent before its
  dep exists on the crates.io index. The cargo adapter now **defers packaging a dependent whose
  `=`-pinned dep is not yet published** into that target's dep-ordered `cargo publish` — so the dep
  is published and index-visible first, then the dependent packages and publishes against it. The
  decision is registry-aware and fail-closed (deferral applies only to a not-yet-published internal
  dep). The outer phase barrier, coordinator-only tagging, and post-tag Homebrew phase are preserved
  (ADR-0002 amendment; ADR-0004 one-target-one-publish-unit intact). This is the fix that lets ossctl
  cut *itself* through its own engine.

## [0.2.0] - 2026-08-06

The release engine can now cut a multi-target, cross-channel release end to end — including
ossctl's own (two crates.io crates + cargo-dist binaries + a Homebrew tap).

### Added
- **`ossctl release list` and `ossctl release abandon`** — inspect release runs (active and
  past) and terminally mark an interrupted run un-resumable, over the event-sourced journal.
  These back the "is a cut already in flight?" gate and interrupted-run recovery.

### Changed
- **`ossctl release cut` now drives a multi-target, cross-channel release end to end.** A
  contract with several publish targets across crates.io, cargo-dist binaries, and a Homebrew
  tap cuts in one run:
  - **Multiple targets in one ecosystem** are published in dependency order (e.g. `ossctl-core`
    before `ossctl`) instead of being rejected.
  - **One plan target = one publish unit** (ADR-0004): the cargo adapter publishes exactly its
    own crate and waits for that crate's workspace dependencies to be index-visible first, so a
    multi-crate cut never double-publishes a shared dependency during the crates.io index lag.
  - **CI-delegated targets are skipped, not failed** — a cargo-dist / gh-releases target whose
    binaries are built by the tag-triggered CI workflow no longer traps the publish phase.
  - **The GitHub Release is left to CI** when a CI-delegated target is present, so the engine and
    cargo-dist's workflow don't both try to create it.
  - **Post-tag Homebrew phase** fetches the tag archive, computes its real `sha256`, and finalizes
    the formula — no hand-filled hash.

### Fixed
- Cargo publishes are **pinned to crates.io** (`--registry crates-io`) and reject any other
  registry, preventing a silently misconfigured host from publishing to the wrong destination
  while ossctl records a crates.io receipt.

## [0.1.2] - 2026-08-05

### Added
- **`ossctl dist generate`** — the release engine now generates a downstream project's
  cross-platform binary-release infrastructure from its `OSS-RELEASE.md` contract: a
  `dist-workspace.toml` (cargo-dist) whose targets come from `distribution.platforms`
  (defaulting to macOS + Linux musl, arm64 + x86_64 — never single-OS) and installers
  from `distribution.installers`, plus the tag-triggered `.github/workflows/release.yml`
  produced via `dist generate` (never hand-templated). This is the piece that makes
  "release through ossctl" real.

## [0.1.1] - 2026-08-05

Cross-platform release: ossctl now installs on Linux as well as macOS.

### Added
- **Cross-platform release artifacts** — prebuilt binaries for macOS (arm64 + x86_64)
  and Linux (arm64 + x86_64, statically linked via `musl` so they run on any distro
  regardless of glibc vintage), plus a `curl … | sh` shell installer, built by cargo-dist
  on tag push.
- `distribution.platforms` in the `OSS-RELEASE.md` contract — a cross-platform binary
  target set that defaults to macOS + Linux, so every project the `/oss-*` family sets up
  produces Linux builds by default.
- `ossctl audit` now flags a `distribution` whose targets omit Linux as a cross-platform
  readiness gap.

### Changed
- The cargo release adapter publishes a multi-crate workspace in dependency order, waiting
  for each crate to appear on the crates.io index before publishing its dependents.
- The Homebrew adapter bootstraps a first formula on an empty tap (create vs. bump).
- Maturity inference no longer conflates version number with release maturity: a
  production-grade pre-1.0 (ZeroVer) repository can reach the `production` tier.
- `/oss-readme` emits cross-platform install instructions (shell installer + prebuilt
  binaries); the contract can model a cargo-dist distribution alongside registry publishes.

### Fixed
- All six ecosystem release adapters now either perform a real publish or return an explicit
  `Unsupported`, instead of a placeholder receipt.

## [0.1.0] - 2026-08-04

First release: published to crates.io, GitHub Releases, and the project's Homebrew tap
by dogfooding ossctl on itself (`/oss-init` → `audit` → release).

### Added
- `ossctl contract show` / `ossctl contract validate` — the single normalizer and
  validator for a project's `OSS-RELEASE.md` release contract (materializes defaults,
  enforces floors).
- `ossctl facts` — deterministic repo-fact detection (ecosystems, workspace members,
  CI, tags, inferred maturity).
- `ossctl audit` — readiness scoring against the gated core (README + LICENSE + CI)
  and the tier-scaled canon.
- `ossctl release {plan,cut,resume,verify,show,list,abandon}` — a resumable, journaled,
  per-ecosystem release-cut state machine with a sealed content-addressed approval plan
  and a remote-is-ground-truth reconcile.
- `ossctl skill {list,install,print}` — the bundled `/oss-*` companion-skill family
  (`oss-init`, `oss-readme`, `oss-ci`, `oss-changelog`, `oss-contributing`,
  `oss-security-policy`, `oss-architecture`, `oss-release`), installable into a repo.
- `ossctl doctor` and `ossctl version` — self-diagnostics and the version/schema surface.
