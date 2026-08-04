# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- oss-changelog:unreleased-start -->
## [Unreleased]
<!-- oss-changelog:unreleased-end -->

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
