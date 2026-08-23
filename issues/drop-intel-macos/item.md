---
created: 2026-08-23
updated: 2026-08-23
type: chore
status: open
priority: high
lane: release-engine
commits:
- hash: caa52c482cc386f0c7a7021089c822b6a437a44b
  summary: 'fix(release): drop Intel macOS prebuilt support'
---

# Drop Intel macOS release target

## Description

Maintainer decision 2026-08-23: x86_64-apple-darwin is no longer built or published. Remove it from every active release path, normalized/default platform requirement, readiness/audit rule, contract, cargo-dist config/workflow, Homebrew formula generation and verification, docs, tests, fixtures, and fleet guidance while preserving historical release evidence. The in-flight v0.11.0 cargo-dist workflow 32652510525 was cancelled while its Intel job remained queued; three supported target builds completed. Finish GitHub Release and Homebrew for macOS arm64 plus Linux musl arm64/x86_64 without claiming Intel support. Record a deliberate replacement of the prior four-platform maintainer policy.

## Resolution

### 2026-08-23T19:25:41Z · @issuectl

Removed Intel macOS from the active three-platform product policy, preserved historical sealed-plan verification, rehearsed the pinned v0.11.0 fallback without external writes, and passed the full gate plus cargo-dist generation/plan checks.

## Reopen Notes — 2026-08-23

_Add rationale for reopening here._

## Comments

### 2026-08-23T19:32:48Z · @conductor

Reopened after fallback execute wrote the correct GitHub Release and Homebrew formula but failed its final byte comparison. Root cause: `gh api --jq ... @base64d` appends a newline, so the observed file has one extra byte. Raw `.content | tr -d "\\n" | base64 --decode` proves the published formula is correct (1042 bytes, sha256 28c40c39f4090448314e5ceae8faf97de35640a9b3e1cf9a4fc1bcb700666e5f) and contains no Intel stanza. Fix the observer and rerun retry-safe verification.
