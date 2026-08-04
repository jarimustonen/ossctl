---
created: 2026-08-04
updated: 2026-08-04
type: task
status: open
priority: high
epic: ossctl-phase4-build
---

# Refresh ossctl's own stale README + add a cross-platform Install section

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

ossctl's README.md is STALE: it says 'Private, early. Not yet published. License to be added' — but 0.1.0 shipped, the repo is public, LICENSE (MIT) exists, and it's on crates.io + a Homebrew tap. Refresh it: correct the Status (public, 0.1.x), fix the License section (MIT, LICENSE present), and ADD a real ## Install section covering BOTH macOS and Linux — cargo (`cargo install ossctl`, source, cross-platform), the shell installer (`curl -LsSf .../ossctl-installer.sh | sh`, available from 0.1.1 via cargo-dist), Homebrew (`brew install jarimustonen/ossctl/ossctl`, works on macOS + Linuxbrew), and prebuilt GitHub-Release binaries (macOS arm64/x86_64, Linux musl arm64/x86_64). ALSO: document in AGENTS.md (operating policy) the standing requirement that ALL software the /oss-* family produces must install on macOS AND Linux — this is /oss-* family canon going forward.
