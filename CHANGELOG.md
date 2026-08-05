# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- oss-changelog:unreleased-start -->
## [Unreleased]
<!-- oss-changelog:unreleased-end -->

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
