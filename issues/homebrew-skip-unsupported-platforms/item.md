---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: in-progress
priority: high
related: ['@homebrew-formula-uninstallable']
lane: release-safety
lane_seq: 0
---

# Homebrew formula renderer fails on non-Homebrew platforms instead of skipping them

## Description

Found by the 0.6.0 cut (2026-08-17), which failed CLOSED at the dist phase:

    Homebrew formula cannot serve unsupported cargo-dist platform `x86_64-pc-windows-msvc`;
    supported platforms are macOS aarch64/x86_64 and Linux musl aarch64/x86_64

The prebuilt-archive formula renderer (landed in 0.6.0, @homebrew-formula-uninstallable) treats every platform in the contract's `distribution.platforms` list as one it must serve, and errors on any it cannot. But a contract legitimately declares platforms for the WHOLE distribution — cargo-dist builds Windows binaries and attaches them to the GitHub Release; Homebrew simply does not serve them. This repository's own contract declares `x86_64-pc-windows-msvc` for exactly that reason.

Correct behaviour: the renderer selects the platforms Homebrew CAN serve (macOS aarch64/x86_64, Linux musl aarch64/x86_64) and IGNORES the rest. A platform Homebrew cannot serve is not an error condition; it is simply not a formula entry.

Fail-closed is still right for the cases it was built for — assets missing, checksum unobtainable. The trigger here is wrong, not the posture. Keep the hard failure when a platform the formula DOES need has no published asset.

State of the 0.6.0 cut: both crates published to crates.io, tag v0.6.0 created and pushed, GitHub Release delegated to CI. Only the Homebrew leg is outstanding; the run is resumable once this is fixed.

Acceptance:
- A contract declaring Windows (or any non-Homebrew-servable platform) alongside servable ones renders a formula covering only the servable ones, no error.
- A contract declaring ONLY non-servable platforms is a clear error (a Homebrew target that can serve nothing is a contract defect).
- A servable platform whose asset is missing still fails closed.
- Regression coverage for all three, using this repository's own four-platform contract shape as the fixture.
