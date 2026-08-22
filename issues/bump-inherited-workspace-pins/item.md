---
created: 2026-08-20
updated: 2026-08-22
type: bug
status: in-progress
priority: high
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
lane: plan-seal
lane_seq: 10
collision: [crates/ossctl-core/src/release/plan.rs, crates/ossctl-core/src/release/bump.rs]
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

## Comments

### 2026-08-20T22:26:59Z · @agent-stint-24

Triaged into bump-exec/20 (stint #24), HIGH, collision on release/plan.rs.

Admitted under the repo issue standard even though it is UNOBSERVED and NOT reachable in ossctl's own contract — ossctl's [workspace.dependencies] holds only external crates, so no internal exact pin is inherited here. It qualifies on the other three grounds, all of which apply at once: the failure is SILENT (local checks resolve through 'path', so the tree stays green), IRREVERSIBLE (a dependent crate publishes to crates.io pinned to the old internal version, and in a multi-crate cut a sibling may already have published before it is observable), and REACHABLE BY A DOWNSTREAM USER (root-workspace inheritance is one of the most common multi-crate Cargo layouts).

Same class as intake-bug-ossctl-d38ddf598fd5, which stint #24 just fixed: plan-time pin discovery not owning the full edit set. That one failed LOUDLY at cut time; this one would not fail at all. Sequenced directly behind it at lane_seq 20 because the fix extends the same discovery path and should build on the reconciled plan/cut agreement rather than race it.

Note the fix likely touches the seal pre-image again — SEAL_VERSION is now 7 after the sibling fix. Check whether widening the pin edit set changes the pre-image; if so that is another deliberate SEAL_VERSION event, not a silent hash change.

Reopen/close condition: close as fixed when a workspace whose member inherits an exact internal pin from root [workspace.dependencies] either has that pin rewritten by the bump, or is refused at PLAN time with an actionable error naming the unsupported shape. Also cover dotted-key and multiline inline declarations, which the same scanner currently misses. Close as wontfix only if inherited exact internal pins are proven unrepresentable in a cuttable contract — record the reasoning.
