---
created: 2026-08-05
updated: 2026-08-16
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
lane: release-hardening
---

# release abandon cannot break a stale single-active-cut lock after a hard-killed run

## Description

All four /llm-review models flagged this as the top issue for release-list-abandon. `release abandon` opens the run under the single-active-cut lock (`Journal::open`). The production lock (`RealJournalStore`) is an `O_EXCL` lock FILE, not `flock` (documented deviation in sys.rs), so a hard-killed `release cut`/`resume` leaves a STALE lock file — the Drop guard never runs. That is exactly the scenario abandon exists for, yet abandon then fails with `cut_in_progress` and cannot mark the dead run abandoned (nor any other run in the repo).

Current mitigation (shipped in the list/abandon change): abandon maps the WouldBlock to an actionable error naming the stale lock file path (<git-common-dir>/ossctl/releases/.lock), so an operator can clear it manually. That is a stopgap, not recovery.

Proposed capability (own design — safety-sensitive): a supervised stale-lock break. Options raised by reviewers: (a) record holder PID+hostname+process-start-identity+run-id in the lock file and add a `--force`/`--break-stale-lock` that verifies the holder is not alive before removing it; (b) move to a real advisory lock (flock/fs2/std File::lock) if MSRV allows — releases on death for free; (c) a dedicated `doctor --fix` stale-lock recovery. Must guard against PID reuse and network/shared filesystems (do NOT delete based on PID alone). Deferred from the list/abandon spinoff to keep that change focused and avoid reworking the locking architecture.

Discovered during release-list-abandon-not-implemented.

## Comments

### 2026-08-10T14:42:17Z · @agent-claude

Hit in the wild during the issuectl 0.8.1 cut (2026-08-10): a foreground 2-min timeout killed the cut wrapper, leaving .lock at <git-common-dir>/ossctl/releases/.lock holding dead PID 910. release abandon refused with cut_in_progress; had to kill -0 the pid, confirm dead, and rm the lock by hand before abandon worked. A dead-PID liveness check (or abandon --force) would avoid the manual rm. See also new @release-cut-publish-noop.
