---
created: 2026-09-06
updated: 2026-09-06
type: task
status: open
priority: normal
---

# Converge Shipshape Taskfleet reference

## Goal

Update Shipshape's one active operating-policy reference from the old fleet repository name to canonical Taskfleet while preserving intentional release-engine fixtures and architecture analogies.

## Authorizing evidence

Taskfleet ADR 0002 E1 owner map: https://github.com/jarimustonen/taskfleet/blob/8b8652a964a1353dc869e89fd541e8cf5b30f1e6/issues/taskfleet-dependent-owner-discovery/owner-map.md (S1-S3).

## Required work

- Update the current fleet release-policy reference in `AGENTS.md` to canonical Taskfleet identity/repository.
- Search the touched scope and classify residual `orchestratectl`/`octl-core` references.
- Preserve release-engine fixtures modeling the historical `octl-core -> orchestratectl` package graph, architecture analogies, canonical Taskfleet formula reconciliation fixtures, stable protocol names, changelog/history, and immutable evidence.
- Run documentation/link checks and the repository gate appropriate to this documentation-only change.

## Acceptance Criteria

- [ ] Current fleet policy names canonical Taskfleet.
- [ ] No fixture/history/protocol identity is accidentally renamed.
- [ ] Required repository checks pass and the issue records the exact commit.
