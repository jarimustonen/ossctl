# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- oss-changelog:unreleased-start -->
## [Unreleased]

### Changed
- **An empty `extra_fields` map is now omitted from canonical JSON** (not emitted as `"extra_fields": {}`),
  applied symmetrically to both the top-level `Contract` and the nested `Distribution` via
  `skip_serializing_if`. A populated `extra_fields` serializes exactly as before. This makes the
  "additive = absent-by-default" migration rule hold literally. No `schema_version` bump (existing
  populated contracts are byte-identical; consumers already tolerate an absent optional key), but the
  release-plan `SEAL_VERSION` bumped `4` → `5` because the sealed pre-image of an empty-`extra_fields`
  contract changed.
<!-- oss-changelog:unreleased-end -->

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

First release — published to crates.io, GitHub Releases, and the `jarimustonen/ossctl`
Homebrew tap by dogfooding ossctl on itself (`/oss-init` → `audit` → release).

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

[Unreleased]: https://github.com/jarimustonen/ossctl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jarimustonen/ossctl/releases/tag/v0.1.0
