---
created: 2026-07-25
updated: 2026-07-25
type: epic
owner: jari
status: open
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@contract-command']
---

# ossctl release engine — plan/cut/resume/verify state machine + adapters + journal (ADR-0002/0003)

## Description

The most program-shaped member and likely its own multi-issue epic. Implement per ADR-0002 + ADR-0003: the ReleaseAdapter trait + enum-backed registry (6 ecosystems, compiled-in, runtime dispatch); the phase-barrier coordinator (dry-run-all→build-all→publish-all→tag-once, coordinator-only tagging, no auto-rollback); the sealed content-addressed plan_id approval seam ('release plan' seals, 'release cut --plan <id>' refuses on drift); PublishReceipt + verify()→{Matches,Conflicts,Missing,Unknown}; the event-sourced JSONL journal under git-common-dir/ossctl/releases/<run_id>/ with append-then-apply atomicity + idempotent reducer + flock; the remote-is-ground-truth resume/reconcile state table; §12 JSONL streaming for 'release cut' + 'release show' progress query. Break into child issues per adapter/phase when picked up. Blocked by contract-command.
