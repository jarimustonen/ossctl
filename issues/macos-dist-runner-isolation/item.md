---
created: 2026-09-27
updated: 2026-09-27
type: bug
reporter: agent
status: open
priority: high
---

# Shipshape self-hosted macOS release installs cargo-dist persistently

_Source: .github/workflows/release.yml_

## Observation

Shipshape itself (GitHub repository `jarimustonen/ossctl`) has `[dist.github-custom-runners] aarch64-apple-darwin = "self-hosted"` and its generated `.github/workflows/release.yml` runs `${{ matrix.install_dist.run }}` unscoped on that build-local-artifacts row. The real v0.12.2 macOS job 102039499720 (run 34219616414, 2026-09-08) ran an `Install dist` step successfully, although its detailed log is no longer available, so do NOT claim direct historical proof of its install path. The exact same cargo-dist 0.33.0 installer on Taskfleet's Hauis self-hosted runner was proven in job 107705797379 to write to `/Users/jari/.cargo/bin` and was safely isolated and proven in Taskfleet v0.11.3 (macOS job 108565483310). Glasspad, issuectl and Project Canon made analogous changes. Shipshape's next release remains vulnerable to recreating unmanaged `~/.cargo/bin/{dist,cargo-dist}` entries if not fixed first.

## Acceptance Criteria

- Install dist on only the self-hosted macOS local-artifact row into an atomic per-job `RUNNER_TEMP` directory; verify `command -v dist` resolves there, fail closed if installer ignores isolation. Preserve Linux matrix and hosted jobs. No automatic removal or modification of pre-existing user Cargo binaries.
- A generated-workflow drift guard regenerates from pristine cargo-dist 0.33.0 output and overlays only the reviewed exception; keep `allow-dirty = ["ci"]` narrow and prove an actual `dist build --artifacts=global` / valid equivalent passes before a release, unlike the initial issuectl CI-only fix which failed late.
- Full pinned-toolchain repo green gate, CI, and one real self-hosted macOS release before declaring runner side effects eliminated; collect before/after Hauis persistent-bin evidence and job log. Do not copy personal runner labels into Shipshape's generic downstream generator.
