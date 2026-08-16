---
created: 2026-08-06
updated: 2026-08-16
type: improvement
status: duplicate
priority: normal
related: ['@cargo-publish-receipt-provenance-resume-safety']
closed: 2026-08-16
---

# cargo-build-disposition-journal

## Description

After the cargo-interleave change a dependent's build phase records TargetBuilt with EMPTY artifacts (packaging deferred to cargo publish), and BuildArtifacts.notes are not journaled. A resumed run / release show cannot distinguish 'built + packaged (.crate)' from 'built, packaging deferred (no artifact)'. Add a durable build disposition (Packaged{artifacts} vs DeferredToPublish{deps}) to the journal. Related to the existing aggregated-build-manifest resume follow-up in coordinator.rs. Surfaced by /llm-review (GPT-5.6, Opus).

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Build disposition journaling is part of the retained cargo receipt/provenance hardening cluster.
