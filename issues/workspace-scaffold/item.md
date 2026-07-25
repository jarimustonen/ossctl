---
created: 2026-07-25
updated: 2026-07-25
type: task
status: done
priority: high
epic: ossctl-phase4-build
commits:
- hash: b939fa7
  summary: scaffold ossctl-core + ossctl-cli workspace; version/doctor + clap taxonomy + CI green
- hash: a8e0be0
  summary: apply llm-review fixes (build.rs freshness, Fs port, doctor/clap polish)
closed: 2026-07-25
---

# Scaffold the ossctl-core + ossctl-cli cargo workspace (ADR-0001)

## Description

FOUNDATIONAL — blocks every other unit. Stand up the two-crate workspace exactly as ADR-0001 §2 specifies: ossctl-core (lib, all logic, injected ports CommandRunner/Clock/IdGen/RegistryQuery/Fs-Git, domain modules contract/facts/audit/release/protocol) + ossctl-cli (clap surface, handlers, output). Wire the clap noun-verb skeleton for the full taxonomy (contract/facts/audit/release/skill/doctor/version) with stub handlers, plus a working 'ossctl version --json' (§10: version, commit, schema_version, supported_schemas, skills[]) and 'ossctl doctor' skeleton (§18). Add CI (fmt+clippy+test) and dist config, near-copyable from issuectl/orchestratectl. No domain logic yet — just the compiling shape. cargo build + cargo test + cargo clippy green.
