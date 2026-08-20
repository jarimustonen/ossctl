---
created: 2026-08-20
updated: 2026-08-20
type: bug
reporter: jari
status: open
priority: high
collision: [crates/ossctl-core/src/release/plan.rs]
lane: bump-exec
lane_seq: 10
provenance: agent-homebase-wrapup
---

# Release bump plan accepts duplicate exact pins then cut fails

## Description

Release bump plan accepts duplicate exact pins then cut fails

## Context

I was releasing `project-canon` 0.6.0 after moving its canonical `cli-canon` skill and templates into the binary-owned skill catalog.

Repository: https://github.com/jarimustonen/project-canon
Feature commit being released: `10a6cd602c02c8e11d7f71d4c8fedc4c9e39ba7c`
Previous release: `v0.5.0`
Installed ossctl: `0.9.0`, commit `4bd2ae389627c39ee925c9969154d019628d7cd4`

At that feature commit, the workspace version was 0.5.0 and `crates/project-canon-cli/Cargo.toml` intentionally declared the same exact internal dependency pin twice:

```toml
[dependencies]
project-canon-core = { path = "../project-canon-core", version = "=0.5.0" }

[dev-dependencies]
project-canon-core = { path = "../project-canon-core", version = "=0.5.0" }
```

The duplicate declarations are intentional: integration tests directly depend on the core crate as well as the CLI's normal dependency.

## Reproduction

From a clean `project-canon` checkout at `10a6cd602c02c8e11d7f71d4c8fedc4c9e39ba7c`:

```sh
ossctl release plan --bump minor --json
```

This succeeded and sealed:

- plan id: `c61f83299026ea74e05b360c97bf904c9ef404b3d144a802ef90f29919d2f073`
- version: `0.6.0`
- reported pin rewrites: one `project-canon-core` rewrite from `=0.5.0` to `=0.6.0`

Then:

```sh
ossctl release cut --plan c61f83299026ea74e05b360c97bf904c9ef404b3d144a802ef90f29919d2f073 --json
```

created run `01M0CDP98NM1HDG9WEW11BE77Y`, entered the bump phase, and failed immediately:

```text
run 01M0CDP98NM1HDG9WEW11BE77Y: bump-phase failed: the `project-canon-core = "=0.5.0"` pin matched 2 declarations — refusing to rewrite an ambiguous pin.
```

No target had been published. I abandoned that run, manually updated the workspace version and both exact pins, refreshed Cargo.lock, finalized CHANGELOG.md, and committed the result as:

`005fafa5ae33ea955a83b76aeefb403f2ff3d6b3 chore(release): prepare 0.6.0`

I then planned and cut the already-bumped tree successfully through the publish/tag phases.

## Observed

`release plan --bump minor` accepted and sealed a plan that `release cut` could not execute. The plan even reported a single pin rewrite although the later bump implementation found two matching declarations.

This violates the evidence-gated plan boundary: the irreversible release command was handed a content-addressed plan whose owned bump phase was already known to be structurally ambiguous.

## Expected

One of these should happen before a plan is sealed:

1. Treat equivalent exact pins in separate Cargo dependency tables as a supported set and rewrite every matching declaration deterministically; or
2. Reject `release plan --bump minor` with the same ambiguity error that cut currently emits.

`release cut` should not discover a deterministic bump-shape error that the immediately preceding plan could have detected from the same sealed tree.

## Analysis pointers

- Compare the pin-discovery/counting logic used by `release plan --bump` with the actual rewrite logic in the cut bump phase.
- The project-canon release journal retains run `01M0CDP98NM1HDG9WEW11BE77Y` and its three events (`run_created`, `phase_entered:bump`, `phase_completed:bump outcome=failed`).
- Public manifest at the feature commit: https://github.com/jarimustonen/project-canon/blob/10a6cd602c02c8e11d7f71d4c8fedc4c9e39ba7c/crates/project-canon-cli/Cargo.toml

## Comments

### 2026-08-20T05:33:01Z · @agent-stint-23

Admitted to the plan (stint #23). Laned bump-exec/10, HIGH, collision on release/plan.rs.

Rationale for HIGH despite a contained blast radius (nothing published; the operator bumped by hand): the class is plan/cut disagreement, not a bump edge case. The sealed plan is the approval artifact, and its whole purpose is that what is approved is executable. Here plan-time pin discovery counted ONE rewrite while the cut-time rewriter found TWO declarations and refused — the same sealed tree, two answers. Any such divergence weakens the seal boundary generally, which is why this outranks an ordinary bump bug.

Fix must reconcile the two code paths, not just relax the cut. The reporter proposes either (a) treat equivalent exact pins across dependency tables as a supported set and rewrite all deterministically, or (b) reject at plan time with the same ambiguity error. Prefer (a) if the pins are provably equivalent: a normal + dev-dependency pin on the same workspace crate is a legitimate, common layout (integration tests depending on core), so refusing it permanently would be a real limitation. Fall back to (b) where equivalence cannot be established.

Collision note: the fix touches release/bump_exec.rs (rewrite) AND release/plan.rs (discovery/count). Check whether the plan-time pin count participates in the seal pre-image — if it does, this is a SEAL_VERSION event per plan.rs evolution rule (currently 6).
