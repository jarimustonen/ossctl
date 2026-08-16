---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: in-progress
priority: normal
lane: release-hardening
lane_seq: 5
---

# Flaky macOS CI: temp_workspace() fixture dirs can collide

## Description

CI on `main` went red on
[run 31965351885](https://github.com/jarimustonen/ossctl/actions/runs/31965351885)
(commit `fd904de`). Only the **macos-latest** test job failed; `ubuntu-latest`,
`msrv (1.85)`, `clippy`, `rustfmt`, `docs`, and the skill↔CLI lockstep job all
passed.

```
thread 'release::bump_exec::tests::applies_version_pin_changelog_and_commits'
panicked at crates/ossctl-core/src/release/bump_exec/tests.rs:177:64:
called `Result::unwrap()` on an `Err` value: Edit(WorkspaceVersionNotFound)

test result: FAILED. 485 passed; 1 failed
```

## This is flaky, not a regression

`fd904de` is a docs-only commit (a filed intake item) and touches no code; the
previous commit `ded4324` was green on the same tree. So the failure is
nondeterministic.

## Root cause

`temp_workspace()` (`crates/ossctl-core/src/release/bump_exec/tests.rs:100`)
names its fixture directory from the pid plus a wall-clock nanosecond reading:

```rust
let dir = std::env::temp_dir().join(format!(
    "ossctl-bump-test-{}-{}",
    std::process::id(),
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos())
));
```

All the tests in this module run as threads of one process, so the pid is
constant and the timestamp is the only thing separating two fixtures. macOS
`SystemTime` granularity is coarser than Linux's — commonly microseconds — so
two tests that enter `temp_workspace()` within the same tick get the **same
directory**. The second one overwrites the first one's `Cargo.toml`, or bumps
the shared workspace version out from under it; the victim's `apply_bump` then
cannot find `version = "0.4.0"` and returns `WorkspaceVersionNotFound`.

That explains all the observed properties: macOS-only, intermittent, and
independent of the commit under test.

## Fix

Make the fixture path unique by construction rather than by clock resolution.
Either is fine:

- a process-wide `static COUNTER: AtomicU64` appended to the name, or
- `tempfile::TempDir::new()`, which also cleans the directory up on drop
  (these fixtures currently leak into `TMPDIR`).

`tempfile` is the better end state, since it fixes the leak at the same time.
The helper is shared by every test in the module, so one change covers them all.
