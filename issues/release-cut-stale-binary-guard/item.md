---
created: 2026-08-10
updated: 2026-08-10
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
---

# release plan/cut runs stale engine code silently when the binary was not built from the current tree (no stale-binary guard)

## Description

**Footgun hit during the 0.2.5 cut (stint #16).** `cargo build --release -p ossctl-cli` silently no-ops (the bin crate is `ossctl`, not `ossctl-cli`), leaving a STALE `target/release/ossctl` (0.2.1). Because 0.2.5's single-source-version change reads the release version from the tree at RUNTIME, `ossctl release plan --version 0.2.5` printed version 0.2.5 and 4 correct targets EVEN FROM THE STALE 0.2.1 BINARY — so a `release cut` would have run old engine code (no self-visibility check, old adapters) while looking correct. The only tell was `ossctl version` reporting 0.2.1. Caught by hand this time.

## Why the existing guard doesn't cover it
The stint #16 `--version`-vs-manifest drift guard compares the `--version` FLAG to the manifest — it does NOT compare the running BINARY's provenance to the tree. A stale binary passes the drift guard cleanly.

## Proposed fix
`ossctl release plan` / `release cut` should detect that the running binary was not built from the current tree and emit a CLI warning (or refuse). Options:
- Compare the binary's compiled-in git commit (already surfaced by `ossctl version` → `commit:`) against `git rev-parse HEAD`; warn/error on mismatch (with an `--allow-stale-binary` escape hatch for intentional cases).
- At minimum a loud WARNING on `plan`/`cut` when `compiled_commit != HEAD`.
This is a structural-safety gap for a tool whose whole story is 'structural, not a human gate'. Not urgent (only bites a mis-built binary) but a clean hardening.

## Acceptance
- [ ] `release plan`/`cut` warns (or errors, behind a flag) when the running binary's compiled commit != tree HEAD.
- [ ] Documented in the cut recipe (AGENTS.md already notes the caveat + the `-p ossctl` gotcha).
