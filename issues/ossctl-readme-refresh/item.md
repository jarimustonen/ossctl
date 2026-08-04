---
created: 2026-08-04
updated: 2026-08-04
type: task
status: fixed
priority: high
epic: ossctl-phase4-build
commits:
- hash: 52be241
  summary: 'docs(readme): refresh status/license + cross-platform Install; AGENTS cross-platform policy'
closed: 2026-08-04
---

# Refresh ossctl's own stale README + add a cross-platform Install section

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

ossctl's README.md is STALE: it says 'Private, early. Not yet published. License to be added' — but 0.1.0 shipped, the repo is public, LICENSE (MIT) exists, and it's on crates.io + a Homebrew tap. Refresh it: correct the Status (public, 0.1.x), fix the License section (MIT, LICENSE present), and ADD a real ## Install section covering BOTH macOS and Linux — cargo (`cargo install ossctl`, source, cross-platform), the shell installer (`curl -LsSf .../ossctl-installer.sh | sh`, available from 0.1.1 via cargo-dist), Homebrew (`brew install jarimustonen/ossctl/ossctl`, works on macOS + Linuxbrew), and prebuilt GitHub-Release binaries (macOS arm64/x86_64, Linux musl arm64/x86_64). ALSO: document in AGENTS.md (operating policy) the standing requirement that ALL software the /oss-* family produces must install on macOS AND Linux — this is /oss-* family canon going forward.

## Outcome (fixed)

- **README.md** refreshed. Status now says **public and shipping**, pointing at the real
  v0.1.0 release (crates.io `ossctl` + `ossctl-core`, GitHub Release `v0.1.0`, Homebrew tap
  `jarimustonen/ossctl`) and CHANGELOG.md. License section now `[MIT](LICENSE)` with the file
  present (no longer "to be added").
- **New `## Install` section** covering macOS + Linux (arm64/x86_64): `cargo install ossctl`
  and `brew install jarimustonen/ossctl/ossctl` presented as the **always-works** paths for the
  current release. The cargo-dist shell installer and prebuilt binaries are described as arriving
  with the **next tagged release (v0.1.1)** — verified against the repo: latest tag is `v0.1.0`,
  which carries no installer/binary assets yet, so those are NOT presented as working today.
  Target list (macOS arm64/x86_64, Linux musl arm64/x86_64) read from `dist-workspace.toml`.
- **AGENTS.md** (root; `CLAUDE.md` symlinks to it) gained a **cross-platform-required** operating
  policy: all `/oss-*`-family software and ossctl itself MUST install on both macOS AND Linux —
  `/oss-*` family canon going forward.
- `cargo fmt --all --check` passes (no Rust changed).
