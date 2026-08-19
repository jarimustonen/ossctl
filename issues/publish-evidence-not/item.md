---
created: 2026-08-17
updated: 2026-08-19
type: improvement
status: open
priority: normal
lane: facts-evidence
lane_seq: 10
---

# publish evidence is not inspectable from the CLI

## Description

The contract normalizer's Cargo `publish` cross-read (`facts::cargo_publish_evidence`, shipped with publish-none-unrepresentable) can produce a HARD normalization floor: 'targets declares a crates.io publish for X, but Y/Cargo.toml forbids publishing'. That error blocks every command (contract show/validate, audit, release plan/cut), and it is triggered by a MANIFEST edit rather than a contract edit — a repo that validated yesterday can fail today because someone added `publish = false` to a crate.

The evidence itself is deliberately off-wire: it is a normalizer input, not part of the `facts` JSON. So when the floor fires and the operator disagrees with it, there is no way to see WHAT ossctl read — which manifests it found, and what verdict (Allowed/Forbidden/Unknown) it assigned each. The only debugging path is reading ossctl's source.

The read is tri-state and covers root + members with inheritance resolved, so a false positive is unlikely by construction; this is about being able to CONFIRM that, not about a suspected bug. Fix shape: expose the per-manifest verdicts, either as a `cargo_publish` block in `ossctl facts --json` (it already carries `packages`, and this is read-only evidence of the same kind) or behind a `contract validate --explain`.

Raised in the multi-model review of publish-none-unrepresentable (Claude Opus 5, §4). Reopen/close condition: close as unnecessary if no false-positive floor is ever reported AND the evidence model stays this small; keep open while the floor is the only fatal diagnostic in ossctl derived from a file the contract does not name.
