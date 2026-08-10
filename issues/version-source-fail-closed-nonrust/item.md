---
created: 2026-08-10
updated: 2026-08-10
type: improvement
status: in-progress
priority: normal
epic: ossctl-phase4-build
related: ['@cut-noop-self-visibility-check']
commits:
- hash: 31b1c2a
  summary: VersionSource capability model; manifest-versioned targets fail closed on missing version
---

# version-drift/self-visibility guards fail OPEN for manifest-versioned non-Rust ecosystems (node/python)

## Description

The 0.2.5 version-drift guard + self-visibility confirm are effective for Rust but SKIP any target whose version can't be read from a manifest — so they no-op (fail OPEN) for npm/PyPI packages. Needs an ecosystem/adapter version-source capability model that distinguishes 'no manifest version BY DESIGN' (homebrew/binary/cargo-dist → legitimately skip) from 'detector failed on a manifest-versioned ecosystem' (→ fail CLOSED). Not a live risk (ossctl publishes only Rust) but a latent gap for the first non-Rust consumer. Filed from the cut-noop /llm-review (stint #16).
