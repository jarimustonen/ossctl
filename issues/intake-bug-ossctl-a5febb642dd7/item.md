---
created: 2026-08-23
updated: 2026-09-03
type: bug
reporter: jari
status: fixed
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:homebase-wrapup-ossctl-cargo-publish-detection-20260823
lane: release-preflight
assignee: agent
commits:
- hash: b4b1f6265720570a51d37e5b0ba3509e888dfd5f
  summary: start release trigger preflight fix
- hash: cf542d8
  summary: parse inline tag publish triggers
- hash: f2b222b
  summary: tighten workflow trigger evidence after review
closed: 2026-09-03
closed_by: agent
---

# Release plan misses tag-triggered Cargo publish workflow

## Description

Release plan misses tag-triggered Cargo publish workflow

## Observed

From a clean orchestratectl main checkout with `.github/workflows/publish-crates.yml` present, this command:

```sh
scripts/ossctl-release.sh plan patch
```

sealed a valid four-target plan but emitted:

```text
cargo-publish-ci delegates crates.io publication for octl-core, orchestratectl to CI, but no tag-triggered Cargo publish workflow was detected under .github/workflows; no directly inspectable Cargo publish path was found in the detected tag-triggered workflows (release.yml).
```

The repository's `.github/workflows/publish-crates.yml` contains `on: push: tags: ['v[0-9]+.[0-9]+.[0-9]+*']` and directly runs `cargo publish -p octl-core` followed by `cargo publish -p orchestratectl`. The resulting v0.5.1 tag later triggered that workflow successfully, and ossctl verification reported both crates.io targets as `matches`.

## Expected

`ossctl release plan` should detect the checked-in tag-triggered Cargo publish workflow and not warn that no directly inspectable Cargo publish path exists. Detection should cover this valid YAML trigger and direct shell publish form without weakening the warning for genuinely missing CI publication paths.

## Evidence

- Plan id: `ee4cdfb863909c73587a9294ccdc613c5b494d81fab061e6b8fd101192b757b3`
- Release journal: `01M0QA6BTN55D9K1YB7QGS83DW`
- Release commit: `f0c52ab232706fb480a51bfd45f2171c6b7aa056`
- Publish workflow run: `32640599433` (success)

## Comments

### 2026-09-03T12:13:53Z · @agent

Root cause confirmed: the trigger detector was an indentation-aware text scan that accepted only a line exactly equal to `tags:`. It therefore discarded the valid inline sequence `tags: [...]` before the existing structured job/`cargo publish -p ...` detector ran. The fix parses the workflow trigger mapping as YAML (including quoted `on` and YAML-1.1 boolean-key representations) while retaining negative checks for branch-only triggers and tag workflows without a direct publish path.

## Resolution

### 2026-09-03T12:52:05Z · @agent

Fixed and verified. Release planning now structurally recognizes the observed inline `push.tags` sequence and direct `cargo publish -p ...` steps with no delegated-publish warning. Empty/malformed filters, branch-only triggers, malformed workflows, invalid reusable triggers, and tag workflows without publish commands remain inconclusive and preserve the warning. `/llm-review` plus `/assess-findings` completed; all five confirmed localized findings were applied. The exact repository green gate passed: fmt, workspace Clippy with warnings denied, workspace tests (637 passed, 9 ignored), workspace build, and rustdoc with warnings denied.
