---
created: 2026-08-05
updated: 2026-08-05
type: improvement
status: open
priority: normal
---

# harden core Journal::open/read against journal identity + structural corruption

## Description

Raised by /llm-review (gpt-5.6-sol, claude-opus-4-7) during the release-list-abandon work. `Journal::open`, `read_run_state`, and `read_run` in `crates/ossctl-core/src/release/journal.rs` reduce the event log WITHOUT validating structural invariants:
- The reduced `state.run_id` is never checked against the requested run id / directory name. A directory `RUN_A/journal.jsonl` whose `RunCreated` declares `RUN_B` can be opened/abandoned via `release abandon RUN_A` (the append goes to RUN_A's dir, but the identity mismatches). `load_state` validates manifest identity; the authoritative log paths do not.
- A non-empty journal need not start with `RunCreated`: a log of only `PhaseEntered` reduces to an in-progress state with empty run_id/version/plan_id, which `list` then shows as an active run and `abandon` will append to.

Proposed: after reduce, validate (in core, one place): first event is `RunCreated`; exactly one `RunCreated`; first seq is 1; seq strictly increasing/unique; `state.run_id == requested run id`; identity fields non-empty. Return `InvalidData` on violation so the CLI maps it to `journal_unreadable`. Real-world likelihood is low (run ids are tool-minted ULIDs; would need a hand-crafted/corrupt journal), which is why it is deferred rather than fixed inline — but it is a cheap, correct hardening of the shared read paths (affects verify/show/resume/list/abandon uniformly). Out of scope for the list/abandon feature per its hard constraints (no broad ossctl-core rework).

Discovered during release-list-abandon-not-implemented.
