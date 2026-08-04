---
created: 2026-07-27
updated: 2026-08-04
type: task
status: fixed
priority: high
epic: ossctl-phase4-build
commits:
- hash: d8518b5
  summary: audit 6 adapter publish() bodies; analysis.md verdict table + completion sketch
- hash: b3b7186
  summary: node release-please Unsupported + real npm-pack tarball name; all 6 adapters honest
closed: 2026-08-04
---

# Audit + complete the 6 ecosystem adapter publish() bodies before a real release cut

_Source: crates/ossctl-core/src/release/adapter_

## Description

From the release-engine /orchestrate campaign (spinoff candidate s-001, report ~/.orchestratectl/runs/01kyfc8jf1x9rbf91kjfwdfssn/report.md). The ReleaseAdapter trait, enum registry, runtime dispatch, dry_run, and verify() are real and tested, but individual adapters' publish() bodies were permitted to be faithful skeletons during the campaign. Before ossctl cuts a REAL release (incl. dogfooding its own publish), audit each of the 6 ecosystem adapters' publish() for completeness and finish any that are stubbed. Depends on the release-engine integration branch (orchestrate/release-engine-2026-07-26) being merged to main first.

## Outcome (fixed 2026-08-04)

All 6 ecosystem release adapters' `publish()` bodies are now REAL or honestly
`Unsupported` — no `SKELETON:` markers remain in any adapter body.

- **cargo / python / go** — already REAL before this unit (`cargo publish`,
  `twine upload --skip-existing`, `goreleaser release`); `cargo-dist` /
  `gh-action-pypi-publish` honestly `Unsupported`.
- **homebrew** — completed on `main` just before this unit (first-formula
  create-vs-bump; url+sha256 threaded from the coordinator).
- **binary** (`manual` / GitHub Releases) — was already finished on `main`
  (commit e25c8e1): `gh release upload <tag> <assets…> --clobber` with the
  asset paths + repo slug threaded from the coordinator's build phase via
  `ReleaseArtifacts`. The audit's SKELETON verdict predated that landing.
- **node** — THIS unit. `release-please` `publish()` now returns
  `AdapterError::Unsupported` (it publishes via CI on merge, not from the host)
  instead of running a representative command + fabricating an npm receipt —
  matching cargo-dist / gh-action-pypi-publish. `npm-publish` / `changesets`
  stay REAL. `build()` now reads the packed tarball's exact name from
  `npm pack --json` (correct for scoped `@scope/pkg` packages) and **fails hard**
  on unparseable output rather than guessing a name (an `/llm-review` finding —
  the earlier fallback reconstructed a wrong name for scoped packages).

Reviewed via `/llm-review` (4 models, 2 rounds); confirmed findings applied,
scope-creep declined and captured as spin-offs (see below). Green gate passes
(fmt / clippy -D warnings / 247 tests / build).

### Auth seam decision
Kept "ambient env is the contract" per the unit brief — no secret-provider
abstraction. node/binary read `GITHUB_TOKEN` / `~/.npmrc` from ambient env like
every sibling adapter.

### Spin-offs proposed (from the review — for the orchestrator to file)
- `node-changesets-batch-receipt` — `changeset publish` is workspace-wide; a
  truthful per-target receipt needs coordinator batch/reconcile semantics.
- `adapter-deferred-publish-outcome` — model CI-driven publishes as a first-class
  `PublishOutcome::Deferred` instead of `AdapterError::Unsupported` (trait-shape
  change; touches the coordinator seam).
- `node-npm-registry-url-and-access` — derive `remote_url` from the target's
  registry (not hard-coded npmjs.com) and add `--access public` for scoped first
  publishes.
