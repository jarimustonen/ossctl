# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- oss-changelog:unreleased-start -->
## [Unreleased]

The first tagged release, `0.1.0`, is being prepared. The entries below are the
initial feature set it will ship.

### Added
- `ossctl contract show | validate` — the single normalizer and validator for a
  project's `OSS-RELEASE.md` release contract (materializes defaults, enforces floors).
- `ossctl facts` — deterministic repo-fact detection (ecosystems, workspace members,
  CI, tags, inferred maturity).
- `ossctl audit` — readiness scoring against the gated core (README + LICENSE + CI)
  and the tier-scaled canon.
- `ossctl release plan | cut | resume | verify | show | list | abandon` — a resumable,
  event-sourced/journaled, per-ecosystem release-cut state machine with a sealed
  content-addressed approval plan and a remote-is-ground-truth reconcile.
- `ossctl skill list | install | print` — the bundled `/oss-*` companion-skill family
  (`oss-init`, `oss-readme`, `oss-ci`, `oss-changelog`, `oss-contributing`,
  `oss-security-policy`, `oss-architecture`, `oss-release`), installable into a repo.
- `ossctl doctor` and `ossctl version` — self-diagnostics and the version/schema surface.
<!-- oss-changelog:unreleased-end -->
