---
created: 2026-08-23
updated: 2026-08-23
type: bug
reporter: jari
status: fixed
priority: high
lane: release-engine
closed: 2026-08-23
commits:
- hash: 3207dfb
  summary: finalize marker-bound changelogs safely
---

# release cut changelog_finalize breaks the marker block and skips fragments

## Description

Observed on the project-canon v0.6.2 engine cut (ossctl 0.10.0, plan c060c98a…, bump.changelog_finalize: true; contract changelog mode=fragment, source=issuectl-trailers, fragment_dir=changelog/fragments). Four defects in one finalize: (1) the dated '## [0.6.2]' header was inserted INSIDE the oss-changelog marker block — unreleased-end ended up below the dated section, so the released section lived inside [Unreleased]; (2) the pending fragment changelog/fragments/agent-skills-terminology.md was neither compiled into the notes nor consumed; (3) no issuectl-trailer compilation ran (a Refs-Issue trailer commit was in range), leaving an empty Added/Changed/Fixed skeleton under the dated header; (4) the broken block propagated verbatim into the cargo-dist GitHub release body, publishing a stray '<!-- oss-changelog:unreleased-end -->' comment. Expected per the /oss-changelog contract: cut the dated header OUTSIDE the markers, restore a clean marker-bounded Unreleased skeleton, and compile notes from trailers + fragments (consuming the fragments). Manual repair on project-canon: commit 19f3bf8 + a gh release edit — useful as the expected-output fixture.

## Resolution

### 2026-08-23T12:54:08Z · @issuectl

Fixed the observed project-canon failure: dated sections now remain outside the marker block, fragment and issuectl-trailer notes compile together, fragments are consumed safely, and marker comments cannot enter release notes. SEAL_VERSION 9 binds the complete finalization intent; legacy stored plans remain loadable. Full green gate and multi-model review/assessment passed.
