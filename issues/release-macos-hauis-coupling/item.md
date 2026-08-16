---
created: 2026-08-05
updated: 2026-08-16
type: improvement
status: wontfix
priority: normal
closed: 2026-08-16
---

# ossctl canon release depends on personal hauis self-hosted runner (macOS aarch64)

## Description

ossctl's dist-workspace.toml routes the macOS aarch64 build to Jari's personal self-hosted 'hauis' runner via [dist.github-custom-runners]. The 0.1.2 release FAILED on first attempt because hauis had a stale http.https://github.com/.extraheader token (HTTP 400) — a documented-but-recurring hauis gotcha. This couples ossctl's CANON release to one personal machine: the release cannot ship without hauis healthy, and the override is explicitly marked non-standard (ossctl never emits it for others). Options: (a) build macOS aarch64 for ossctl's OWN release on GitHub's hosted macos-14 (arm64) runner — decoupling canon ossctl from hauis while keeping hauis available for other repos' fast builds, (b) keep hauis but add a health/precheck step. Context: this was flagged during the stint-11 effort to move personal-environment concerns to homebase; the hauis runner infra itself now lives in homebase issue cross-repo-release-standardisation.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Personal runner coupling is a homebase or infrastructure concern, not active ossctl product work. Closing here.
