---
created: 2026-08-17
updated: 2026-08-19
type: bug
status: open
priority: normal
lane: verify-seam
lane_seq: 40
---

# delegated PyPI/npm targets are verified against GitHub Release assets

## Description

The coordinator's verify barrier dispatches a CI-delegated target by registry.
`homebrew` observes the tap formula; `crates.io` observes the registry index (added by
`release-ci-publish-mode`); everything else falls through to
`verify_delegated_release`, which polls the **GitHub Release** for
`<package>-<triple>.tar.xz` archives.

That fallthrough is wrong for the two delegated identities that upload to a package
registry rather than to GitHub: `gh-action-pypi-publish` (PyPI) and `release-please`
(npm). Such a target would be observed at the wrong destination and, after the bounded
20-minute wait, journal `Missing` — failing an otherwise-healthy cut whose publish in
fact succeeded.

**Reachable here:** not in ossctl's own contract (rust-only), but reachable by any
consumer repo that declares a delegated Python or node target — which is a shape the
contract explicitly supports and the adapter layer explicitly models. **Damage:** a red
cut after a successful, irreversible publish, plus a 20-minute wait to get there; it
contradicts the ADR-0002 verify amendment's guarantee that a target is observed *at its
destination*. Loud rather than silent, and recoverable via `resume`, hence a bug rather
than a blocker.

**Suggested shape:** route `npm` / `pypi` / `testpypi` delegated targets through the same
registry-index observer `crates.io` uses (`verify_delegated_registry`), leaving
`gh-releases` on the Release-asset observer. Note the production `RegistryQuery` today
wires only `rust` and `node`, so a delegated PyPI target would observe `Unknown` (an
honest "cannot look") rather than a false `Missing` — that is the correct interim
outcome, and wiring the PyPI client is the follow-on half.

**Reopen/close condition:** close as fixed when a delegated npm target is verified
against the npm registry and a delegated PyPI target reports `Unknown`-or-observed rather
than a Release-asset `Missing`. Close as wontfix only if delegated non-crates registry
targets are removed from the contract's supported surface.
