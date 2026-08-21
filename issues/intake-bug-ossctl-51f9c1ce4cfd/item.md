---
created: 2026-08-21
updated: 2026-08-21
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:d2d40ad8-c9c6-439e-a39c-da8b68092d92
---

# homebrew/binary target verification fails: missing during cut, unknown …

## Description

homebrew/binary target verification fails: missing during cut, unknown afterwards, formula is present

Declaring a cargo-dist-owned Homebrew target makes `release cut` fail its verify phase, even though the formula is published correctly. This re-introduces the "successful release reports as failed" class that `verify-gh-release-missing` just fixed — on a different target.

Observed on `project-canon` v0.6.1 with **ossctl 0.10.0**, run `01M0HVWEYD8712DCXJ8FHMCX79`.

## Context: this is the newly-declared target

`project-canon` just added its Homebrew channel to the contract (it had been publishing to Homebrew via cargo-dist for weeks without declaring it). The declared target is:

```yaml
- {ecosystem: binary, package: project-canon, registry: homebrew, adapter: cargo-dist}
```

`ecosystem: binary` + the project-level package name were chosen deliberately, because ossctl 0.10.0's delegated Homebrew verification passes `target.package` to `verify_tap_formula`, which fetches `Formula/<package>.rb`. cargo-dist writes `Formula/project-canon.rb` (`formula = "project-canon"`), not the Rust package name.

## The good news first

`verify-gh-release-missing` is genuinely fixed. In this same run, `rust:project-canon-cli:gh-releases` verified `matches`. So this is a **distinct defect on the homebrew/binary adapter**, not a regression of the old one.

## Symptom

During the cut, verify polled ~20 minutes and then failed:

```
{"kind":"target_verified","target":"binary","outcome":"missing"}
{"kind":"phase_completed","phase":"verify","outcome":"failed"}
error: release_failed … verify-phase failed on target `binary`: binary is missing at its destination
```

Re-running the read-only reconcile after everything settled gives a **different** outcome for the same target:

```
$ ossctl release verify 01M0HVWEYD8712DCXJ8FHMCX79 --json
{"target":"binary","package":"project-canon","outcome":"unknown",
 "detail":"the release destination could not be observed (network or command failure)"}
summary: reconciled 4, matches 3, conflicts 0, missing 0, unknown 1
```

The other three targets verify `matches` in both reads.

## The formula is unambiguously present and correct

```
$ curl -s -o /dev/null -w "%{http_code}" https://raw.githubusercontent.com/jarimustonen/homebrew-project-canon/main/Formula/project-canon.rb
200
$ curl -s …/Formula/project-canon.rb | grep -m1 version
  version "0.6.1"
```

- Tap repo visibility: **PUBLIC**, default branch `main`.
- `Formula/project-canon.rb` → 200. `project-canon.rb` → 404. `Formula/project-canon-cli.rb` → 404. So the declared `package: project-canon` is the correct key, and the only path that exists.
- The GitHub Release `v0.6.1` published at 09:57:58 and the cargo-dist workflow succeeded in 3m30s; the formula was in place well before verify gave up (the `binary` target was declared missing ~18 minutes after the formula existed — the same timing signature the old gh-releases bug had).

## Why the two different outcomes matter

`missing` (during cut) and `unknown` / "could not be observed (network or command failure)" (afterwards) are contradictory verdicts about an identical, settled, publicly reachable artifact. That suggests the observation path fails and is being reported as `missing` in one code path and `unknown` in the other, rather than a wrong lookup key returning a truthful "not found". Worth checking whether the cut-time and reconcile-time homebrew observers share an implementation, and whether an observation failure is being coerced into `missing`.

An observation failure should never be reported as `missing`: `missing` asserts a fact about the registry, and acting on it (retry, manual reconcile) is exactly the dangerous response.

## Impact

Any repo that correctly declares its cargo-dist Homebrew target gets a guaranteed false-red cut, plus a 20-minute wait before it. Since the declaration is the *right* thing to do, this currently punishes correct configuration — and pushes operators back toward leaving the channel undeclared and unverified, which is the original defect.

## Expected

The homebrew/binary target verifies `matches` when the tap formula exists at `Formula/<package>.rb` at the released version; an observation failure reports `unknown`, never `missing`.
