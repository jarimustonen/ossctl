---
created: 2026-08-23
updated: 2026-08-23
type: chore
status: open
priority: high
lane: release-engine
---

# Drop Intel macOS release target

## Description

Maintainer decision 2026-08-23: x86_64-apple-darwin is no longer built or published. Remove it from every active release path, normalized/default platform requirement, readiness/audit rule, contract, cargo-dist config/workflow, Homebrew formula generation and verification, docs, tests, fixtures, and fleet guidance while preserving historical release evidence. The in-flight v0.11.0 cargo-dist workflow 32652510525 was cancelled while its Intel job remained queued; three supported target builds completed. Finish GitHub Release and Homebrew for macOS arm64 plus Linux musl arm64/x86_64 without claiming Intel support. Record a deliberate replacement of the prior four-platform maintainer policy.
