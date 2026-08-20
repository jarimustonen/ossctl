---
created: 2026-08-20
updated: 2026-08-20
type: bug
status: untriaged
priority: normal
provenance: ai-review
source_ref: orchestratectl:01m0fathfnk4dexmz93kqnkeag/review-finding:sha1:caa04773eaf5fac25945826ffc382b42b96466bb
review_source: ai-review
originating_run: 01m0fathfnk4dexmz93kqnkeag
originating_run_kind: spinoff
assessment_classification: CONFIRMED
assessment_outcome: SPIN_OFF
review_confidence: HIGH
review_severity: high
review_target: d36be5d-and-revised-diff
labels:
- ai-review-model:gemini-3.1-pro-preview
- ai-review-model:gpt-5.6-sol
- ai-review-model:claude-opus-5
- ai-review-model:deepseek-v4-pro
---

# Release bump can leave inherited exact workspace pins stale

## Description

## Description

Release bump discovery only models exact internal pins declared directly in single-line member dependency tables. It does not own exact pins inherited from a root `[workspace.dependencies]` table, and it can miss dotted-key or multiline inline dependency declarations.

A common workspace shape is:

```toml
# root Cargo.toml
[workspace.package]
version = "0.4.0"

[workspace.dependencies]
core = { path = "crates/core", version = "=0.4.0" }

# member Cargo.toml
[dependencies]
core = { workspace = true }
```

During an engine-owned minor bump, the root workspace package version can become `0.5.0` while the inherited exact pin remains `=0.4.0`. Local checks resolve through `path`, so they may stay green. If `core@0.4.0` already exists on crates.io, publishing the dependent can also succeed, leaving the newly published dependent crate tied to the old internal version. In a multi-crate cut, another crate may already have published irreversibly before this is observable.

## Expected

Before sealing a bump plan, ossctl must prove that every exact internal workspace pin is represented in the sealed edit set. It should either support root workspace inheritance and other valid Cargo declaration forms through the shared parser, or refuse unsupported exact-pin shapes at plan time with an actionable error.

## Review evidence

Confirmed while reviewing the repeated-pin seal-boundary fix. The member pin scanner intentionally covers direct dependency tables; root `[workspace.dependencies]` is not an edit target. Existing order-edge parsing also recognizes dotted local edges without preserving a split dotted `version` requirement.
