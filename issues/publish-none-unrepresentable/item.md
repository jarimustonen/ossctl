---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: normal
lane: contract-engine
lane_seq: 20
---

# normalizer cannot express a publish-nothing rust repo (targets: [] force-expands to crates.io)

## Description

Cross-repo audit finding (2026-08-17), documented in intakectl's OSS-RELEASE.md rationale since 2026-08-06 (ossctl 0.1.2 era, still true): for a rust ecosystem the normalizer force-expands an empty targets: [] into a crates.io/cargo-publish target — there is no registry: none / publish-none representation. A private, never-published repo (intakectl: haapa-resident service, deploy.sh + systemd, Cargo.toml publish = false) cannot author a truthful contract; the normalized contract shows a phantom crates.io target and the repo is stuck at status: draft. Silent-wrong-result class (the normalized output asserts a publish surface that must never exist) + blocks a downstream repo from adopting the contract at all. Fix: allow targets: [] (or an explicit publish: none) to survive normalization for an ecosystem, with audit/plan/cut treating it as no-publish-targets; validate should also cross-read Cargo.toml publish = false as supporting evidence. Reopen intakectl's contract approval once shipped.
