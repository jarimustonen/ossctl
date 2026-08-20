---
created: 2026-08-17
updated: 2026-08-20
type: bug
status: open
priority: high
lane: verify-seam
lane_seq: 10
---

# release verify reports a published GitHub Release as missing

## Description

The verify phase of `release cut` reports a `gh-releases` target as `missing` even when the
GitHub Release exists, is published (not draft), and carries all cargo-dist assets. This turns
a fully successful release into a reported failure.

Observed against `project-canon` on 2026-08-17, on **both** releases cut that day (v0.4.0 and
v0.5.0), and it still reproduces on demand afterwards. Confirmed present in **ossctl 0.9.0**
(originally hit on 0.8.0; 0.9.0's gh-releases verify-barrier work does not fix it).

## Impact

`release cut` exits non-zero with `release_failed` and tells the operator to "reconcile the
registries manually before retrying" — for a release where **everything actually landed**:
both crates on crates.io, the tag pushed, the GitHub Release published with binaries, and the
Homebrew formula updated. An operator (or an autonomous agent) who trusts the exit code will
believe a good release is broken, and may retry or attempt manual reconciliation against
crates.io, which is irreversible.

The failure is also slow: verify polls for ~20 minutes before giving up, so each cut ends with
a 20-minute wait and then a false alarm.

## It is a lookup bug, not a timing race

This is the important part. The obvious hypothesis is that verify polls before CI has finished
publishing. That is **not** what happens.

For v0.5.0 (run `01M08P4D4HK25MRQXDE0XDW9NJ`):

- verify entered at `ts 1786998112`
- the GitHub Release was published at **20:24:14** (≈ `ts 1786998254`)
- verify declared the target `missing` at `ts 1786999318`

So the release existed for roughly **18 minutes of the 20-minute polling window** and was never
observed. v0.4.0 (run `01M08H40373DF571PXY979934D`) shows the identical signature.

Decisive test: run the read-only reconcile **long after** the release is fully published and
settled. It still reports missing:

```
$ ossctl release verify 01M08P4D4HK25MRQXDE0XDW9NJ --json
{
  "target": "rust:project-canon-cli:gh-releases",
  "ecosystem": "rust",
  "package": "project-canon-cli",
  "version": "0.5.0",
  "outcome": "missing",
  "detail": "the registry does not report this version as published"
}
```

summary: `reconciled: 3, matches: 2, conflicts: 0, missing: 1, unknown: 0` — the two crates.io
targets verify fine; only `gh-releases` fails.

Meanwhile the release is plainly there:

```
$ gh release view v0.5.0
tag:        v0.5.0
draft:      false
published:  2026-08-17T20:24:14Z
asset:      dist-manifest.json
asset:      project-canon-cli-aarch64-apple-darwin.tar.xz
asset:      project-canon-cli-aarch64-unknown-linux-musl.tar.xz
asset:      project-canon-cli-x86_64-unknown-linux-musl.tar.xz
asset:      project-canon-cli-installer.sh
...
```

Since a settled, long-published release still reads as missing, the defect is in **how the
gh-releases adapter looks the release up**, not in when it looks.

## Suspected cause

Unconfirmed, but the shape of the data suggests a name/tag mismatch in the lookup:

- the git **tag** is `v0.5.0` (with the `v` prefix)
- the release **title** is `0.5.0 - 2026-08-17` (no `v`, plus a date suffix)
- the target is keyed by **package** `project-canon-cli`, while the release is named for the
  **project**, and the tap formula is `project-canon`

A lookup keyed on the bare version, or on the package name, or on an exact title match would
miss this release while the crates.io lookups (which key on package + version, and are correct)
succeed. Worth checking the adapter's query against all three of tag / title / package naming.

Note this repo has a related precedent: `project-canon` 0.3.2 had to fix cargo-dist naming the
Homebrew formula after the *package* rather than the *project*. The same package-vs-project
naming ambiguity may be biting here.

## Expected

`release verify` / the verify phase observes a published GitHub Release for the run's tag and
reports `matches`, so a successful multi-target cut exits zero.

## Acceptance

- Verify resolves the GitHub Release for the run's tag regardless of release title formatting
  and of package-vs-project naming.
- A regression test covers a release whose tag carries a `v` prefix and whose title does not.
- Re-verifying `project-canon` runs `01M08H40373DF571PXY979934D` (v0.4.0) and
  `01M08P4D4HK25MRQXDE0XDW9NJ` (v0.5.0) reports `matches` for the `gh-releases` target.

## Notes

Both `project-canon` runs remain journaled as `in_progress` / `in_flight` because of this false
failure, even though both releases fully succeeded. Whatever fix lands should also make those
runs reconcilable (`release verify` / `release resume`) rather than leaving them permanently
in-flight.

## Comments

### 2026-08-19T04:36:17Z · @agent-stint-23

Lane-derivation note (stint #23 orchestrator, unverified hint — not a diagnosis): the cut-time observer adapters/binary.rs::observe_release_assets keys on the git tag v{version} and looks correct. The error text in this report ("the registry does not report this version as published") is release/reconcile.rs wording, and that path routes gh-releases through a generic *registry* query keyed on receipt.registry_url. Suggest starting the investigation in reconcile.rs rather than in the tag lookup.

### 2026-08-20T05:32:44Z · @agent-stint-23

FOURTH occurrence, and new evidence (from duplicate intake-bug-ossctl-09cd3c1d03d0, project-canon v0.6.0, 2026-08-19):

- Reproduced on ossctl 0.9.0, commit 4bd2ae389627 — the released binary. 0.9.0 gh-releases verify work does NOT fix this.
- Run 01M0CDRH6CBM574MRFMQTCR9W2, plan de0d83794b6a. crates.io both matched; only gh-releases returned missing after the ~20 min wait.
- External proof of existence at the time of the false missing: release published 2026-08-19T07:15:41Z with dist-manifest, installer, formula, source archives, checksums and macOS/Linux arm64+x86_64 archives; cargo-dist workflow run 32226769657 green; crates.io and the Homebrew tap both at 0.6.0.
- NEW SYMPTOM worth fixing alongside: `ossctl release list --json` showed THREE earlier project-canon runs stuck in_progress on the same delegated gh-releases shape. So the fault does not only fail a cut, it leaves accumulating unresolved runs in the journal.
- The operator had to `release abandon` a fully-landed release to clear it.

Reporter naming the affected package: project-canon-cli (binary) vs project-canon (project/tag/tap) — consistent with the package-vs-project lookup hypothesis already recorded.

