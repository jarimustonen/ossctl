---
created: 2026-08-05
updated: 2026-08-05
type: task
status: in-progress
priority: normal
epic: ossctl-phase4-build
related: ['@release-engine-cut-cargo-dist-flow']
---

# coordinator GitHub Release vs cargo-dist release.yml ownership

_Source: review of release-engine-cut-cargo-dist-flow_

## Description

ADR-0002's coordinator-only tag phase creates the GitHub Release (RealTagger.create_github_release -> gh release create). But ossctl's OWN release flow uses cargo-dist, whose tag-triggered release.yml ALSO creates/finalizes the GitHub Release and uploads the cross-platform binaries. With an engine-driven cut (Track B), the coordinator would create the Release first (from the tag), then cargo-dist's workflow fires on the same tag and may (a) fail on 'release already exists', or (b) upload into the coordinator-created release, or (c) the run is marked Completed with an empty Release until CI finishes.

This is pre-existing (the tag phase always created a Release) but becomes live the moment ossctl cuts itself through the engine — exactly what release-engine-cut-cargo-dist-flow unblocks. Investigate cargo-dist's actual behavior against a pre-existing GitHub Release for the tag, and decide ownership: either (1) the coordinator should NOT create a Release when a cargo-dist/CI-delegated target is present (let CI own it), or (2) cargo-dist config must be set to upload into an existing release. Add an end-to-end check before the first real engine-driven 0.2.0 cut.

Surfaced by /llm-review (openai).
