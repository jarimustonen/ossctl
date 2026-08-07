---
created: 2026-08-06
updated: 2026-08-07
type: improvement
status: done
priority: normal
epic: ossctl-phase4-build
commits:
- hash: 8428e1b
  summary: 'feat(sys): unify registry queries behind one ureq http_get seam'
- hash: 4d2b5e0
  summary: 'harden(sys): review fixes for the registry http_get seam'
- hash: '2858123'
  summary: 'issue: file deferred /llm-review findings'
closed: 2026-08-07
---

# Unify registry queries behind a lightweight HTTP client instead of shelling to curl/npm

## Description

Surfaced by the 4-model `/llm-review` of the harvested crates.io `RegistryQuery`
wiring (`release-publish-registry-query-not-wired`, commits 8fc1e85/483ce0b). The
`RealRegistryQuery` port now has two arms, each shelling to a different external
binary: `node` → `npm view … versions --json`, `rust` → `curl` against the
crates.io sparse index. Three of the four reviewers (Gemini, OpenAI, Opus)
independently argued the shell-out-per-ecosystem approach does not scale and
recommended taking a single small, synchronous, non-async HTTP dependency (e.g.
`ureq`, ~200 KB, no `tokio`) in the **cli** crate (not `ossctl-core`, so the pure
domain stays dependency-free).

## Why (the trade-offs the current approach carries)

- **`curl`/`npm` are undeclared hard runtime deps.** A minimal CI image without
  `curl` turns every `rust` publish-state probe into `unknown` (fail-closed, so
  safe — but it silently blocks the engine cut). Absence is handled with a named
  `Err`, but the dependency is real and unlisted.
- **The `npm` timeout gap stays open.** The `rust` arm bounds itself with
  `curl --max-time`; the `npm` arm still has no wall-clock timeout (`std` has none
  on `Command::output`). A real HTTP client closes both uniformly.
- **The status-marker hack (`\n__OSSCTL_HTTP_CODE__:` + `rsplit_once`) exists only
  because `curl` mixes body and status on one stream.** A native client returns a
  typed `(status, body)` and the hack disappears.
- **Every new ecosystem (PyPI, RubyGems, Go proxy) either spawns another brittle
  shell-out or forces the HTTP dep anyway** — cheaper to unify now, behind one
  `http_get(url) -> io::Result<(u16, Vec<u8>)>` seam.

## Scope / non-goals

Pure refactor of the transport under `RealRegistryQuery`; the fail-closed contract
(`Ok(vec![])` = missing, non-empty = published, `Err` = unknown) and every existing
test must hold unchanged. Does **not** add per-version checksum capability — that
is `cargo-publish-receipt-provenance-resume-safety`. Low priority: the current
shell-out works and is proven end-to-end against live crates.io.

Refs-Issue: release-publish-registry-query-not-wired
