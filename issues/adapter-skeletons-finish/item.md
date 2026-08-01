---
created: 2026-08-01
updated: 2026-08-01
type: feature
status: in-progress
priority: high
epic: ossctl-phase4-build
related: ['@adapter-publish-completeness']
blocked_by: ['@adapter-artifact-threading']
commits:
- hash: e25c8e19426496d3b3a041f4d7239592871a5ea3
  summary: finish binary+homebrew publish bodies (real receipt URL + tarball sha256)
---

# Finish binary + homebrew publish() bodies using threaded artifact inputs

_Source: crates/ossctl-core/src/release/adapters/{binary,homebrew}.rs_

## Description

Complete the two SKELETON adapters now that adapter-artifact-threading provides real inputs. binary.rs: real GitHub-Release asset upload (uses threaded asset paths). homebrew.rs: real bump-formula-pr with the threaded tarball URL + sha256. Remove the SKELETON: markers. These touch two distinct adapter files (binary.rs, homebrew.rs) plus possibly a small shared seam — can be one worker doing both. Blocked on adapter-artifact-threading (needs its threaded inputs). Production code: run /llm-review + /assess-findings before merging. Note: python/node/go finishing is out of scope here and stays under the parent adapter-publish-completeness umbrella.
