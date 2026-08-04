---
created: 2026-08-04
updated: 2026-08-04
type: bug
reporter: jari
status: open
priority: normal
---

# release cut rejects >1 target per ecosystem, but contract validate + /oss-init accept/emit them

_Source: release engine / contract_

## Description

A contract with two rust/crates.io targets (a workspace with two publishable crates, e.g. octl-core + orchestratectl) PASSES `ossctl contract validate` (valid:true, targets:2) and is exactly what /oss-init's default expansion + rationale produce for a multi-crate Rust workspace. But `ossctl release cut` fails immediately: {"error":{"code":"invalid_plan","message":"two targets resolve to the same journal id 'rust'; multiple targets in one ecosystem are not supported by a single cut"}}. The generator/validator and the release engine disagree — a validated, APPROVED contract cannot be cut. EXPECTED: either contract validate rejects >1 target per ecosystem (fail fast at generation with a clear message), or release cut supports N targets per ecosystem (publish workspace members in dependency order). Workaround this session was to collapse to one target. ossctl 0.1.0.
