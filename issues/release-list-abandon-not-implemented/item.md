---
created: 2026-08-04
updated: 2026-08-05
type: bug
reporter: jari
status: in-progress
priority: normal
---

# release list/abandon return not_implemented, but /oss-release skill directs their use

_Source: release engine_

## Description

During an orchestratectl 0.1.0 release cut, `ossctl release list --json` and `ossctl release abandon <run-id> --reason ... --json` both return {"error":{"code":"not_implemented","message":"... (workspace scaffold)"}}. But the /oss-release SKILL (cut-release mode) directs the agent to run `ossctl release list` FIRST to detect an in-flight run before sealing a second plan, and points at resume/verify/abandon for recovery. With list unimplemented the 'already-active run?' gate is a dead end; an interrupted run cannot be abandoned. Also `release resume` returned resume_conflict demanding --allow-unverified even though nothing was published (build failed pre-publish), so reconciliation UX is rough. EXPECTED: implement list/abandon (at least enough for the skill's gate + recovery), or have the skill degrade gracefully when they are stubs. ossctl 0.1.0.
