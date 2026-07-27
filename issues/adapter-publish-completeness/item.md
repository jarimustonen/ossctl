---
created: 2026-07-27
updated: 2026-07-27
type: task
status: open
priority: high
epic: ossctl-phase4-build
---

# Audit + complete the 6 ecosystem adapter publish() bodies before a real release cut

_Source: crates/ossctl-core/src/release/adapter_

## Description

From the release-engine /orchestrate campaign (spinoff candidate s-001, report ~/.orchestratectl/runs/01kyfc8jf1x9rbf91kjfwdfssn/report.md). The ReleaseAdapter trait, enum registry, runtime dispatch, dry_run, and verify() are real and tested, but individual adapters' publish() bodies were permitted to be faithful skeletons during the campaign. Before ossctl cuts a REAL release (incl. dogfooding its own publish), audit each of the 6 ecosystem adapters' publish() for completeness and finish any that are stubbed. Depends on the release-engine integration branch (orchestrate/release-engine-2026-07-26) being merged to main first.
