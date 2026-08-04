---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: open
priority: high
epic: ossctl-phase4-build
---

# Distribution block needs a cross-platform target set with a Mac+Linux default

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

REQUIREMENT (user directive): all OSS software the /oss-* family produces MUST install on both macOS AND Linux. The contract's `Distribution` block ({adapter, gh_releases, installers, homebrew_tap}) has NO platform-target field, so a downstream project can't express WHICH platforms its binaries cover and nothing guarantees Linux. KEYSTONE fix: add a platform-target set (Rust target-triples) to `Distribution` in crates/ossctl-core/src/contract/schema.rs + normalize.rs, defaulting to a CROSS-PLATFORM set — macOS (aarch64+x86_64) and Linux (aarch64+x86_64, prefer musl for static/glibc-free), optionally Windows. Normalize/validate the triples; preserve the canonical-JSON schema-versioned contract (bump schema_version if the shape breaks; a new optional field with a cross-platform default should be additive — follow the project rule). This default is what makes every downstream cargo-dist distribution cover Linux by default. Blocks #2/#3/#5 which read this field.
