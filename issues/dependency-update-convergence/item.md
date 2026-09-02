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

## Agent Runs

### 2026-09-02T05:58:05Z · @dependency-update-convergence-worker

Dependency convergence implementation is complete in run `01m1g96442f6sdr28b9w0agf2g`; leave this issue **OPEN** until the conductor verifies merged `main` CI and disposes every GitHub PR.

Per-proposal disposition:

- **#1 dtolnay/rust-toolchain 1.85 → 1.100 — REJECTED.** The ref is a Rust compiler selector, not an independent action release tag. Rust 1.100.0 does not exist (`rustup` returned HTTP 404), the PR desynchronizes `rust-toolchain.toml`, and it incorrectly replaces the explicit Rust 1.85 MSRV job. All PR checks failed. Reopen only when Rust 1.100.0 is actually published and a proposal updates the exact repository pin plus all normal CI refs together while retaining the separate 1.85 MSRV job.
- **#2 actions/cache v4 → v6 — INCLUDED.** All authored CI registry-cache steps use v6; paths and keys are unchanged, including the separate MSRV key. Official notes provide security/dependency maintenance and the Node 24 runtime is compatible with hosted runners.
- **#3 actions/checkout v4/v6 → v7 — REJECTED AS PROPOSED; safe subset included.** Authored CI and crate-publish workflows use v7, but the generated release workflow must remain at cargo-dist 0.32.0's canonical v6. Forcing v7 into generated output was reproduced to make `dist plan` refuse the release workflow as stale. Both v6 and v7 use Node 24; self-hosted `hauis` is Runner.Listener 2.337.0. Reopen the release-workflow portion when a stable cargo-dist generator emits checkout v7. Do not hand-edit `release.yml`.
- **#5 actions/download-artifact v4 → v8 — INCLUDED.** cargo-dist 0.32.0 canonically emits download v8 paired with upload v7. Artifact names, paths, and `merge-multiple` topology remain generator-owned; v8's fail-closed digest mismatch behavior improves release safety.
- **#6 actions/attest-build-provenance v2 → v4 — INCLUDED VIA THE OFFICIAL SUCCESSOR.** cargo-dist 0.32.0 canonically emits `actions/attest@v4`; the old action's v4 notes identify it as a wrapper over this successor. Subject-only mode officially auto-generates SLSA build provenance, and OIDC/attestation permissions are now scoped to the build job.
- **#7 clap 4.6.4 → 4.6.6 — INCLUDED.** Targeted lock update; no source changes required.
- **#8 ureq 3.3.0 → 3.4.0 — INCLUDED.** Targeted lock update, including required `ureq-proto`/`base64` changes; current usage does not implement the newly sealed extension trait.
- **#9 toml 0.8.23 → 1.1.4+spec-1.1.0 — INCLUDED WITH API ADAPTATION.** TOML 1.1 changed `Value`'s `FromStr` behavior to parse a single value. The full-document call site now uses `toml::from_str`; five failing contract drift tests became green.
- **#10 toml_edit 0.23.10+spec-1.0.0 → 0.25.13+spec-1.1.0 — INCLUDED.** Existing edit APIs remain compatible; converging with toml 1.1 removes duplicate toml_edit/toml_datetime/winnow generations.

Additional necessary convergence: cargo-dist was upgraded from 0.28.2 to 0.32.0 so action upgrades remain generator-owned. Its canonical workflow also updates upload-artifact to v7, disables persisted checkout credentials, scopes attestation permissions, and strengthens the host gate. `dist plan` succeeds and plans ten artifacts across the unchanged supported matrix (macOS arm64; Linux musl arm64+x86_64). The authored crate-publish checkout also disables persisted credentials.

Verification completed: official action metadata/release notes; self-hosted runner/OS check; targeted lock generation; `cargo update --dry-run`; locked metadata; final-tree Rust 1.85 check; actionlint (only unchanged cargo-dist-generated SC2086/SC2129 diagnostics suppressed); cargo-dist 0.32.0 canonical generation and `dist plan`; `/llm-review` (four models, two cross-review rounds) plus `/assess-findings`; and the exact full green gate (`fmt`, Clippy `-D warnings`, tests, build, rustdoc `-D warnings`).

Conductor after merge: verify `main` CI is green, close #1 and #3 with the rejection/reopen rationale above, close #2/#5/#6/#7/#8/#9/#10 as superseded by the merged convergence commit, confirm no listed proposal remains open, then close this issue. Do not merge Dependabot branches after this convergence commit lands.
