---
created: 2026-09-02
updated: 2026-09-02
type: task
status: open
priority: normal
lane: dependency-updates
---

# Converge outstanding dependency updates

## Description

Evaluate and converge every currently open Dependabot proposal through one tested dependency-update round. Scope: Rust dependencies toml_edit (#10), toml (#9), ureq (#8), clap (#7); GitHub Actions attest-build-provenance (#6), download-artifact (#5), checkout (#3), cache (#2), and dtolnay/rust-toolchain (#1).

Do not merge proposals blindly. Group compatible updates, adapt code and lockfiles where required, preserve the repository-pinned Rust/CI synchronization policy, run the full green gate and main CI, then merge or close each Dependabot PR so no proposal remains ambiguous. Record any deliberately rejected major update and its reopen condition.
