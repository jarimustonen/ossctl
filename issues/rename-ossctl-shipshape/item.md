---
created: 2026-08-23
updated: 2026-08-23
type: chore
status: open
priority: high
size: XL
lane: repo-wide-rename
---

# Rename ossctl to Shipshape

## Description

## Goal

Rename the public product, CLI, crates, repository-facing documentation, distribution surfaces, and bundled skill family from `ossctl` to **Shipshape**.

Shipshape's product promise is: it gets repositories release-ready and then publishes them safely, reproducibly, and with destination-backed verification.

## Locked naming decisions

- Product and CLI: `shipshape`
- Core Rust crate: `shipshape-core`
- Bundled skill namespace: `/shipshape-*`
- The `ctl` suffix is removed.

The bundled catalog becomes:

- `/shipshape-init`
- `/shipshape-readiness`
- `/shipshape-readme`
- `/shipshape-ci`
- `/shipshape-changelog`
- `/shipshape-contributing`
- `/shipshape-security-policy`
- `/shipshape-architecture`
- `/shipshape-dist`
- `/shipshape-release`

## Scope

- Inventory every public and internal `ossctl` identity: binary and package names, crates, Rust module/API references, help and JSON output, environment/path names, contract prose, journal/plan storage paths, bundled skills, documentation, CI, cargo-dist, Homebrew, crates.io, GitHub URLs, installers, examples, tests, fixtures, snapshots, issue/ADR references, and release metadata.
- Design and execute a safe migration for the already-public `ossctl` 0.10.x installation and release channels. Explicitly decide compatibility behavior for the old binary, crate names, skill names, and persisted git-common-dir state rather than silently stranding existing users or resumable runs.
- Rename the product and skill family consistently to Shipshape.
- Preserve canonical JSON and journal compatibility unless a deliberate schema/version migration is documented and tested.
- Update the release contract and distribution configuration so Shipshape can release itself on all currently supported channels and platforms.
- Update user-facing positioning to emphasize repository readiness plus safe, resumable, verified releases.

## Acceptance criteria

- `shipshape` is the canonical executable and public product name.
- Published/package naming and the compatibility policy for `ossctl` are documented and tested.
- All ten bundled skills install under `/shipshape-*`; old skill handling is deliberate and documented.
- Existing resumable release runs and sealed plans have a tested migration or an explicit, actionable compatibility refusal.
- README, AGENTS guidance, ADR context, CLI help, generated artifacts, CI, cargo-dist, Homebrew instructions, and release metadata consistently use Shipshape where historically appropriate.
- Historical records remain accurate rather than being blindly rewritten.
- The full repository green gate passes.
- A machine-convergence and release plan covers replacement of the installed `ossctl` command without leaving stale persistent binaries or skills.

## Implementation guidance

This is a repository-wide rename with a large collision surface. Implement it as a planned migration, not a mechanical global search-and-replace.
