---
created: 2026-08-17
updated: 2026-08-17
type: feature
status: done
priority: high
related: ['@homebrew-formula-uninstallable', '@release-verify-delegated-github-release']
lane: release-safety
lane_seq: 2
closed: 2026-08-17
commits:
- hash: 45cfb51
  summary: verify barrier and destination observers
- hash: b64a71d
  summary: finish post-hoc verification and regression coverage
---

# release verify must confirm the Homebrew tap carries the released version

## Description

## The gap

Nothing checks that a tool's Homebrew tap actually carries the version that was just released.
The engine treats "the tap write returned success" (or, where the leg is delegated, "the phase
was skipped as delegated") as proof the Homebrew channel worked. Neither is proof, and in
practice the channel has been failing silently across the family.

Three independent instances, found within one day, each with a different local cause and all
with the same symptom — a green release and a tap that does not carry the release:

1. **This project.** The engine writes the formula itself, and the formula it writes cannot
   install (`cargo install` against a virtual workspace manifest). Six releases reported the
   Homebrew leg green; the maintainer's own machine is still on the last version installed
   before the engine took over the write. See `homebrew-formula-uninstallable`.
2. **A project whose contract declares a tap but no homebrew target.** `plan` seals only the
   crates.io targets and the tap leg is dropped with no warning; the formula was updated by hand
   for two consecutive releases. See `intake-feature-ossctl-73e870268475`.
3. **A project that delegates the leg to cargo-dist with publishing disabled.** Its tap is three
   versions behind (stuck at 0.11.0 while 0.14.0 shipped). The disable was recorded as temporary,
   pending a credential that in fact already existed.

The causes differ; the missing capability is the same. **A publish target that cannot be observed
after the fact is not a publish target — it is a hope.** The engine already applies this
principle to crates.io: it confirms the version reached the index before journaling a receipt.
The Homebrew leg has no equivalent.

## What to build

`ossctl release verify <run>` should, for a declared Homebrew target or a declared
`distribution.homebrew_tap`, read the tap's formula and assert it names the released version.
Distinguish clearly:

- **carries the released version** — the leg genuinely worked;
- **carries an older version** — the leg silently failed or was skipped, whatever the phase
  reported;
- **absent / unreadable tap** — cannot verify, reported as such rather than as success.

A delegated leg (cargo-dist publishing the formula from CI) is not verifiable at cut time — CI
has not finished — so the same distinction the delegated GitHub Release needs applies here:
delegated-and-observed, delegated-and-pending, delegated-and-failed-after-a-timeout. This issue
and `release-verify-delegated-github-release` are the same shape of problem for two different
targets and should probably be designed together.

Verifying the version is the floor, not the ceiling: a formula can name the right version and
still be uninstallable, which is exactly instance 1 above. Whether verification should go further
— fetching the formula's declared artifact and checking it resolves, or asserting the formula
shape — is worth deciding rather than assuming. Version equality alone would have caught
instances 2 and 3 but not 1.

## Acceptance

- `release verify` reports the Homebrew leg's real state, per the distinctions above, for both
  engine-written and delegated taps.
- A tap lagging the released version is a reported failure, never silence.
- An unreadable or absent tap is reported as unverifiable, never as success.
- The decision on how deep verification goes (version equality vs. installability) is recorded
  with its reasoning.

## Scope note

This is the generic engine capability and belongs here. Re-enabling or reconfiguring any
particular downstream project's release pipeline belongs to that project.

## Resolution

### 2026-08-17T07:36:06Z · @issuectl

Mandatory verification now fetches the destination formula through the bounded HTTP seam and checks the ownership marker, released version, and every declared platform URL/checksum stanza. Unknown and stale observations cannot complete a cut.
