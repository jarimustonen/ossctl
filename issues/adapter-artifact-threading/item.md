---
created: 2026-08-01
updated: 2026-08-01
type: feature
status: open
priority: high
epic: ossctl-phase4-build
related: ['@adapter-publish-completeness']
---

# Thread release artifact paths (asset paths + tarball URL/sha256) from coordinator into adapters

_Source: crates/ossctl-core/src/release/coordinator.rs + adapters/mod.rs seam_

## Description

The single cross-cutting gap from adapter-publish-completeness/analysis.md: the coordinator must thread concrete release artifacts (built binary/asset paths, and the published tarball URL + sha256) into each adapter's publish() via the shared EffectCtx/AdapterTarget seam. Both SKELETON adapters (binary, homebrew) are blocked on this — binary needs asset paths to upload; homebrew's bump-formula-pr needs the tarball URL + sha256. REAL adapters (cargo/python/go) unaffected. This is shared release-engine logic (coordinator.rs + adapters/mod.rs) — run ALONE, do not parallelise (true shared-logic hot file). Production code: run /llm-review + /assess-findings before merging.
