---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: fixed
priority: high
epic: ossctl-phase4-build
blocked_by: ['@distribution-cross-platform-targets']
closed: 2026-08-04
---

# oss-readme must emit cross-platform install snippets (shell installer + gh-release binaries)

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

REQUIREMENT: downstream READMEs must show how to install on BOTH macOS and Linux. Today `oss-readme` (crates/ossctl-cli/skills/oss-readme/SKILL.md) emits install snippets only from registry `targets[]` (cargo install / brew install). When the contract has a `distribution` block it must ALSO emit: a shell-installer snippet (`curl -LsSf .../<pkg>-installer.sh | sh` — works on macOS+Linux), a GitHub-Release prebuilt-binary path, and a note that the platform coverage includes Linux + macOS (from the distribution's cross-platform targets, issue distribution-cross-platform-targets). Note brew also works on Linuxbrew. Depends on distribution-cross-platform-targets for the target list.

## Outcome

Fixed in `crates/ossctl-cli/skills/oss-readme/SKILL.template.md` (the bundled-skill
source; embedded via `include_str!`, so the §17 lockstep gate re-checks it — no
hand-edited generated copy exists).

- Phase 0 field-list now names `distribution` (`{adapter, gh_releases, installers[],
  homebrew_tap, platforms[]}`) as a rendered contract field.
- Phase 1 gained a **"Cross-platform install from the `distribution` block"** subsection
  that instructs emitting, whenever `distribution` is non-null and **additive** to the
  preserved registry `targets[]` snippets:
  - a **shell installer** one-liner (`curl -LsSf .../<pkg>-installer.sh | sh`) only when
    `installers[]` includes `shell` — with a conditional PowerShell note gated on a
    `*-windows-*` triple being present in `platforms[]`;
  - a **prebuilt GitHub-Release binary** path only when `gh_releases` is true, with a
    platform-coverage note derived **strictly** from `platforms[]` target-triples (e.g.
    macOS arm64/x86_64, Linux static-musl arm64/x86_64) — never claiming an OS/arch the
    set does not list;
  - a note that `brew install` covers **macOS and Linuxbrew**.
- Ordering rule added: always-works registry/source paths (cargo/brew) first, then shell
  installer, then prebuilt binaries. Registry snippets explicitly preserved.

Full green gate passed: fmt, clippy (`-D warnings`), `cargo test --workspace` (incl.
`skill_lockstep.rs`, 7 tests green), `cargo build --workspace`. No code logic changed
(generation-guidance prose only) so `/llm-review` was not run — the quality-bar minimum
(lockstep + build green) is met.
