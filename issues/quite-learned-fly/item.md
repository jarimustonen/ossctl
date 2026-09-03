---
created: 2026-09-03
updated: 2026-09-03
type: improvement
status: untriaged
priority: normal
provenance: other
provenance_detail: orchestratectl autonomous implementation run
source_ref: orchestratectl:01m1khdx02qsn4fcr0z2sxpm0w/task
originating_run: 01m1khdx02qsn4fcr0z2sxpm0w
originating_run_kind: spinoff
---

# Conform skill installer to canon section 15

## Description

## Goal

Bring Shipshape's bundled companion-skill catalog and installer into full conformance with project-canon 0.8.0 §15 for Claude, pi, Codex, and `all`.

## Scope

Verify the released canon afresh, then implement catalog capability metadata, default-all installation, canonical `--target`, retained `--dest` compatibility, `--dry-run`, explicit force/no-clobber behavior, self-contained Codex prompts, focused tests, documentation, snapshots, and changelog updates.

## Verification

Run project-canon 0.8.0 doctor/review §15 checks and the repository's exact Rust green gate. Use disposable target/HOME directories only.

## Agent Runs

### 2026-09-03T12:05:15Z · @pi

Implemented the project-canon 0.8.0 §15 catalog and installer contract. The exact Rust green gate passed, and disposable-directory §15 capability/default/all/single-agent/target/dry-run verification passed. project-canon doctor still reports the pre-existing out-of-scope §24 stale-deferral finding. project-canon review runtime probes expect unwrapped top-level JSON and therefore report a false §15 `skills[]` gap against Shipshape’s established `{schema_version,data,warnings}` envelope; direct inspection at `.data` and focused tests verify the complete §15 payload and behavior.
