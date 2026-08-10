---
created: 2026-08-06
updated: 2026-08-10
type: feature
status: done
priority: normal
epic: ossctl-phase4-build
commits:
- hash: be08674
  summary: honor explicit empty targets:[] as authoritative (Option B) + tests
- hash: cda19c8
  summary: review-driven hardening tests + no-bump doc
closed: 2026-08-10
---

# Contract can't express 'publish target none' for a registry ecosystem

## Description

## Problem

There is no way to declare a project that should be **version-tracked + changelogged but never published to any registry** (a private/internal service deployed by its own script). For a `rust` (or any registry) ecosystem, the normalizer **force-expands a registry target that cannot be removed**.

## Reproduce (ossctl 0.1.2)

Given `OSS-RELEASE.md` frontmatter with `ecosystems: [rust]`, every one of these still normalizes to a `crates.io`/`cargo-publish` target:

- `targets: []`               → `ossctl contract show` reports `targets: [{registry: crates.io, adapter: cargo-publish}]`
- `distribution: {adapter: manual, gh_releases: false}` (targets omitted) → same crates.io target remains
- a top-level `publish: none` field → ignored (lands in `extra_fields`), crates.io target remains

The `registry` enum has no non-publishing value:
```
targets[0].registry 'none' invalid — one of [crates.io, npm, pypi, testpypi, gh-releases, proxy.golang.org, homebrew]
```
(`manual` also rejected.)

## Expected

A first-class way to say 'no registry publish' — e.g. `registry: none` accepted (target present but non-publishing), or an empty `targets: []` respected as authoritative (not re-expanded) when the author sets it explicitly, or a top-level `publish: none` honored by the normalizer.

## Impact / workaround

Consumer (intakectl, a private haapa-only service): `OSS-RELEASE.md` says publish target none, but the normalized contract every `/oss-*` member reads contains a crates.io target. Worked around with `Cargo.toml` `publish = false` (so `cargo publish` hard-fails) + a documented caveat in the release doc — but the machine-readable contract still misrepresents intent, and a future automation consuming the normalized targets could try to publish. The tool should be able to represent 'never publish'.

## Decision (Jari, 2026-08-10) — Option B

**Chosen mechanism: B — respect an explicitly-set empty `targets: []` as authoritative** (do
NOT re-expand it into a default crates.io target). Rejected: A (`registry: none` enum value) and
C (top-level `publish: none`). Implementation must distinguish *explicit empty* from *absent*
(serde `Option<Vec<Target>>`: `None` = omitted → keep the ecosystem-default expansion; `Some([])`
= author's authoritative "never publish"). Ripples to every `/oss-*` member that reads normalized
targets — an empty target set must be honored downstream, not treated as "misconfigured". Land via
a reviewed worktree (production code → `/llm-review` before merge).
