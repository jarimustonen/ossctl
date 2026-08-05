---
created: 2026-08-04
updated: 2026-08-05
type: feature
status: in-progress
priority: normal
---

# Release engine generates dist-workspace.toml / release.yml from distribution.platforms

_Spin-off from `oss-release-cross-platform-dist`: that issue documented the
contract→cargo-dist mapping as the binding default; this issue BUILDS the
generator that reads it._

## Description

The `ossctl` release engine does not yet **generate** a downstream project's
binary-release infrastructure. Today it consumes only `distribution.homebrew_tap`
(threaded into the Homebrew adapter in `crates/ossctl-core/src/release/plan.rs`);
it never emits `dist-workspace.toml` or the tag-triggered
`.github/workflows/release.yml`. The `oss-release` skill prose
(`crates/ossctl-cli/skills/oss-release/SKILL.template.md` → "Release-infra
generation — cross-platform by default") now documents the required mapping, but
no code produces the config.

Build the generator so the engine (or a dedicated `ossctl` subcommand) emits:

- `dist-workspace.toml` with `[dist] targets` from `distribution.platforms`
  (cross-platform default: macOS arm64+x86_64 + Linux musl arm64+x86_64 — never
  macOS-only) and `[dist] installers` from `distribution.installers` (ensure
  `shell` for the Unix curl-installer covering Mac+Linux).
- `.github/workflows/release.yml` via `dist generate` from that config (never
  hand-authored).
- Mirror `ossctl`'s own `dist-workspace.toml` reference shape (pinned
  `cargo-dist-version`, github hosting/ci, attestations, `pr-run-mode = "skip"`).

Related engine gaps: `cargo-adapter-workspace-publish`,
`homebrew-adapter-first-formula`, `gh-release-ci-workflow`. Reference the
`AGENTS.md` cross-platform hard requirement. The documented default from the
spin-off source issue is the contract this generator MUST honor.

