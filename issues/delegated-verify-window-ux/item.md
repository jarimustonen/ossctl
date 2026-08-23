---
created: 2026-08-17
updated: 2026-08-23
type: improvement
status: in-progress
priority: normal
lane: verify-observers
lane_seq: 30
collision: [crates/ossctl-core/src/release/coordinator.rs]
---

# delegated verify windows are serial, silent, and per-target

## Description

Each delegated observer (`verify_delegated_release`, `verify_delegated_homebrew`,
`verify_delegated_registry`) takes its own `start = clock.now_unix()`, so every delegated
target gets a fresh `DELEGATED_RELEASE_VERIFY_TIMEOUT_SECS` (20 min) budget, and none of
them emits anything to the progress sink while polling.

**Reachable here:** yes. A realistic contract — cargo-dist gh-releases + cargo-dist
homebrew + a `cargo-publish-ci` crates.io target — has a worst case of ~60 minutes of
blocking, silent wall time when CI is broken. **Damage:** the operator sees a frozen
terminal, assumes a hang, and `^C`s a run that is mid-verify with the tag already pushed.
That is recoverable (`release resume`), but the resumed run restarts the full window from
zero. Pre-existing for the Release/tap legs; the crates.io leg added by
`release-ci-publish-mode` makes three.

**Suggested shape:** compute ONE barrier deadline in `verify_phase` and pass it to every
observer, and emit a sink event per poll (or every Nth) naming what is being waited for
and how long is left — e.g. `waiting for crates.io to show tool@1.0.0 (4m12s / 20m)`.

**Reopen/close condition:** close as fixed when a three-delegated-target cut cannot block
longer than one window in total and streams progress while it waits. Close as wontfix only
if the per-target budget turns out to be load-bearing (a slow tap write genuinely needing
its own 20 minutes after a slow Release).

## Comments

### 2026-08-21T09:49:30Z · @agent-stint-24

Scope caveat added during the stint #24 DAG audit (verified against code, not the title).

Confirmed STILL REAL: coordinator.rs has four separate 'let start = ctx.clock.now_unix()' sites (lines 1596, 1650, 1684, 1821), so every delegated observer gets its own full DELEGATED_RELEASE_VERIFY_TIMEOUT_SECS budget, and the poll loops call ctx.clock.sleep with no ProgressSink emission at all.

BUT this is the weakest of the five open issues, and it should NOT be implemented blind. It sits directly behind verify-delegated-run-state in the same lane and on the same file. Any fix that teaches verify to distinguish pending from failed will necessarily restructure these same poll loops and will almost certainly have to emit progress while it waits. Re-evaluate this issue AFTER that one lands: it may shrink to a one-line deadline hoist, or vanish entirely.

Its damage is also the mildest in the set — an operator staring at a frozen terminal and possibly ^C-ing a mid-verify run. That is recoverable via resume, unlike the silent/irreversible failures the other issues describe. Kept because the failure is real and reachable, not because it is urgent.

Reopen/close condition unchanged; add: close as obsolete if verify-delegated-run-state's fix already delivers a single barrier deadline plus per-poll progress.
