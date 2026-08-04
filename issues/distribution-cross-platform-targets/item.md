---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: in-progress
priority: high
epic: ossctl-phase4-build
---

# Distribution block needs a cross-platform target set with a Mac+Linux default

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

REQUIREMENT (user directive): all OSS software the /oss-* family produces MUST install on both macOS AND Linux. The contract's `Distribution` block ({adapter, gh_releases, installers, homebrew_tap}) has NO platform-target field, so a downstream project can't express WHICH platforms its binaries cover and nothing guarantees Linux. KEYSTONE fix: add a platform-target set (Rust target-triples) to `Distribution` in crates/ossctl-core/src/contract/schema.rs + normalize.rs, defaulting to a CROSS-PLATFORM set — macOS (aarch64+x86_64) and Linux (aarch64+x86_64, prefer musl for static/glibc-free), optionally Windows. Normalize/validate the triples; preserve the canonical-JSON schema-versioned contract (bump schema_version if the shape breaks; a new optional field with a cross-platform default should be additive — follow the project rule). This default is what makes every downstream cargo-dist distribution cover Linux by default. Blocks #2/#3/#5 which read this field.

## Resolution (fixed)

**Field name:** `distribution.platforms` — a `Vec<String>` of Rust target-triples. `targets` was already taken (the registry-publish `Vec<Target>`), and `binary_targets` reads awkwardly next to it; `platforms` is unambiguous and matches how the cargo-dist ecosystem talks about the set.

**Default (omitted OR empty `[]` → this set):**
```
aarch64-apple-darwin
x86_64-apple-darwin
aarch64-unknown-linux-musl
x86_64-unknown-linux-musl
```
macOS + Linux, `musl` over `gnu` (statically-linked pure-Rust CLI has no glibc-version cliff). Windows is a deliberate omission — a bonus a repo opts into by listing `x86_64-pc-windows-msvc` explicitly, never the default. The default always contains ≥1 Linux triple, so every distribution covers Linux by default. Empty falls back to the default too (mirrors empty-`targets` → expand), so an empty list can never silently drop Linux.

**Validation:** each triple is lexically validated (`is_target_triple`: 2–4 `-`-separated `[a-z0-9_]` components) — rejects malformed/uppercase/injection strings while accepting every standard triple; the OS component stays inspectable so the downstream `audit-cross-platform-gap` issue can flag Linux-less explicit sets. Explicit lists are de-duplicated preserving author order. This issue only guarantees the field is present + well-formed; it does NOT implement the Linux-coverage audit.

**schema_version decision: STAYS 1 (additive).** `platforms` is a NEW optional key added inside the already-optional `distribution` block — no existing field is renamed, removed, or re-meant. A reader keying into `distribution.adapter`/`installers`/… is unaffected; the new key never existed before, so there is no prior shape for it to be incompatible with (a pure addition, exactly the case `KNOWN_SCHEMA_VERSION`'s doc and the migration rule call additive-safe). Registry-only contracts (`distribution: null`) are wholly unchanged. The one nuance — omitted resolves to a populated set rather than `null` — does not make it breaking: it only ever ADDS a key, never alters an existing one.

Also updated the oss-init `SKILL.template.md` §4 field table so the generator documents `platforms` + its default (kept in lockstep with the binary).
