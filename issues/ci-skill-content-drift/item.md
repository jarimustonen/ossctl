---
created: 2026-09-28
updated: 2026-09-28
type: bug
status: fixed
priority: normal
closed: 2026-09-28
closed_by: agent
---

# CI skill tests fail after skill content drift

## Description

CI on main at `04a1ba5` is red in three skill tests: https://github.com/jarimustonen/ossctl/actions/runs/36365300217. Latest release v0.12.3 predates this commit; no later green CI is visible.

## Reproduction

`cargo test --locked --workspace` and `cargo test --locked -p shipshape-cli --test skill --test skill_lockstep` (GitHub Actions Ubuntu/macOS). The failing job log reports:

```
thread 'every_skill_body_uses_only_canonical_command_and_skill_names' panicked at crates/shipshape-cli/tests/skill.rs:151:9:
shipshape-release contains a retired command, crate, or skill reference
thread 'skill_install_refuses_newer_on_disk' panicked at crates/shipshape-cli/tests/skill.rs:538:5:
assertion failed: !std::fs::read_to_string(&path).unwrap().contains("stale")
thread 'skill_install_shipshape_dist_writes_distribution_manual' panicked at crates/shipshape-cli/tests/skill.rs:398:9:
skill must not instruct personal infrastructure or global installation: github-custom-runners
```

The lockstep job also fails; the Ubuntu/macOS workspace test jobs fail. These tests are checking shipped skill content and installer behavior. The published skill text and the canonical/neutral-content test expectations have diverged; the on-disk newer-file test also needs investigation of its fixture/installer behavior. Do not weaken the checks solely to make CI green.

## Quick Test

Reproduce each named failing skill test locally; update the shipped `shipshape-release` and `shipshape-dist` content to match the canonical neutral interface, fix the newer-file installer assertion at its cause, then run `cargo test --locked --workspace` and the skill lockstep tests. Confirm CI green on main.
