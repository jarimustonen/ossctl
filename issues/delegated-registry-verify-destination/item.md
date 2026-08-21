---
created: 2026-08-17
updated: 2026-08-21
type: bug
status: open
priority: normal
lane: verify-observers
lane_seq: 20
collision: [crates/ossctl-core/src/release/coordinator.rs]
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

## Comments

### 2026-08-21T09:49:30Z · @agent-stint-24

Blast-radius clarification from the stint #24 DAG audit (verified against code).

Confirmed STILL REAL and NOT subsumed by the 0.10.0 work. coordinator.rs verify_phase dispatches delegated targets as: (Adapter::CargoPublishCi, _) -> verify_delegated_registry; (_, Registry::Homebrew) -> verify_delegated_homebrew; everything else -> verify_delegated_release (GitHub Release assets). The adapter enum does carry ReleasePlease, NpmPublish, GhActionPypiPublish and Twine, so all four fall through to the Release-asset observer.

Narrower than the title suggests, though, and the next implementer should know it: the FALSE MISSING only bites npm today. The issue's own analysis notes the production RegistryQuery wires only rust and node, so a delegated PyPI target reports Unknown ('cannot look') rather than a false Missing — an honest outcome, and the correct interim state. Wiring the PyPI client is a separate follow-on, not part of closing this.

Not reachable in ossctl's own contract (rust-only). Kept on the downstream-user limb of the issue standard: the contract explicitly models these adapters, so a consumer repo can declare one and get a red cut after a successful irreversible publish.

Sequenced ahead of delegated-verify-window-ux (lane_seq 20 vs 30) in the stint #24 re-lane: this is a correctness bug, that one is ergonomics.
