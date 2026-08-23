---
schema_version: 1
status: draft
maturity: mvp
ecosystems: [rust]
targets:
  - {ecosystem: rust, package: rg, registry: crates.io, adapter: cargo-publish}
versioning: semver
release:
  model: gated
  layout: single
license: Proprietary-Acme    # FLOOR VIOLATION: not a valid SPDX id, but a registry target exists
health_badges: [ci, registry, license]
---

> **DRAFT — human review required before use.** NEGATIVE FIXTURE — must fail the
> "a target.registry requires a valid SPDX license" floor.
