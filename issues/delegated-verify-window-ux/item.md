---
created: 2026-08-17
updated: 2026-08-19
type: improvement
status: open
priority: normal
lane: contract-engine
lane_seq: 20
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
