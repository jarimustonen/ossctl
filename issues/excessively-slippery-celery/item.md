---
created: 2026-11-27
updated: 2026-11-27
type: bug
status: fixed
priority: normal
provenance: other
provenance_detail: release driver observation
source_ref: taskfleet:01m8en8aph52v6dfs5c3gmzcg3/observation:fragment-trailer-duplicate
originating_run: 01m8en8aph52v6dfs5c3gmzcg3
originating_run_kind: spinoff
lane: unlaned
closed: 2026-11-27
commits:
- hash: 59b6dfe
  summary: let a fragment replace its issue's trailer line
---

# Fragment-mode changelog finalize duplicates entries that have both a fragment and an issuectl trailer

## Description

Observed in the project-canon v0.10.0 cut (`changelog: {mode: fragment, source: issuectl-trailers}`, bump owned by the engine).

The finalized `## [0.10.0]` section lists every change twice: once as the hand-written fragment from `changelog/fragments/<slug>.md`, and again as a one-line entry generated from the commit trailer for the same issue slug. Example from the published CHANGELOG.md:

```
### Added

- `new --repo-name <name>` sets the repository slug used by the ... hooks separately from `--name`, ...

- Allow repository and CLI names to differ in project-canon new (`intake-feature-project-canon-13a630659b61`).
```

The same pattern appears under Changed/Fixed for `intake-bug-project-canon-61b6e91f85cb` and `review-declared-profile`. The fragment file name equals the issue slug in each case.

Expected: when a fragment exists for an issue slug, the trailer-derived line for that slug is suppressed (the fragment README in generated repos says fragments are for changes "you want worded explicitly", i.e. they replace the generated line). Also, the generated lines are separated from the fragment bullets by blank lines, which renders as a loose list.

Impact: every fragment-mode + issuectl-trailers release ships duplicated, inconsistently worded notes in the public changelog and GitHub Release body.
