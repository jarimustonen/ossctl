---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
---

# gh-releases/cargo-dist: no real multi-platform release workflow

## Description

Found during ossctl's OWN 0.1.0 self-cut (stint #9). cargo-dist config is minimal ([package.metadata.dist] dist = true) with no installers/tap/CI release workflow. The self-cut's GitHub Release + binary asset were built and uploaded BY HAND for a single platform (aarch64-apple-darwin) via 'gh release create'. For a proper release, wire cargo-dist (or a release.yml GitHub Actions workflow) to build cross-platform binaries on tag push and attach them, and optionally generate/push the Homebrew formula (which would also address homebrew-adapter-first-formula for the cargo-dist path). Not blocking — the release exists — but the binary matrix is currently macOS-arm64 only.
