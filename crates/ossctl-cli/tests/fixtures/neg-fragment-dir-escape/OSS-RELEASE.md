---
schema_version: 1
status: draft
maturity: mvp
ecosystems: [rust]
targets:
  - {ecosystem: rust, package: rg, registry: crates.io, adapter: cargo-publish}
versioning: semver
changelog:
  mode: fragment
  source: manual
  fragment_dir: /etc          # FLOOR VIOLATION: absolute path escapes the repo, trivially "exists"
release:
  model: gated
  layout: single
license: MIT
health_badges: [ci, registry, license]
---

> **DRAFT — human review required before use.** NEGATIVE FIXTURE — must fail the
> fragment_dir-must-be-relative-inside-repo floor (an absolute/escaping path would defeat
> the producer-existence check).
