---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: fixed
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@distribution-cross-platform-targets']
closed: 2026-08-04
---

# audit should flag a distribution missing Linux targets as a readiness gap

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

Enforce the 'all OSS software installs on macOS AND Linux' policy in the readiness engine. `ossctl audit` (crates/ossctl-core/src/audit/) should flag a `distribution` block whose target set omits Linux (no *-unknown-linux-* triple) as a readiness gap — recommended at mvp, stronger at production. Makes the cross-platform requirement self-checking rather than convention-only. Depends on distribution-cross-platform-targets (needs the target field to inspect).

## Resolution

`audit/mod.rs::cross_platform_gap` (called from `producer_gaps`) inspects a present
`distribution` block's `platforms` and checks **both** OSes independently:

- No `*-unknown-linux-*` triple → `distribution-linux` gap.
- No `*-apple-darwin` triple → `distribution-macos` gap (symmetric — the policy is macOS AND
  Linux; a Linux-only set is as much a gap as a macOS-only one).
- Missing both (e.g. a Windows-only set) → both gaps.

Classifiers `is_linux_triple` / `is_darwin_triple` are substring checks. `-unknown-linux-`
is deliberate (not `-linux-`): it excludes `*-linux-android`, a Linux-kernel target that is
not a desktop-Linux install target. No gap for a registry-only repo (`distribution: None`) or
a set already covering an OS.

Severity is `Recommended` / `Category::Producer` at all tiers (the audit reserves `Blocking`
for the gated core; same idiom as the `security-policy` gap), with the detail wording
escalating to "required by the cross-platform install policy" at production. Member routes to
`oss-init` — the sole writer of `OSS-RELEASE.md`, where `platforms` lives.

Went through `/llm-review` (gemini/openai/anthropic): the symmetric-macOS check and the
"declares" (not "builds") wording were applied from consensus findings; the severity-model /
category / member findings were assessed and kept (local precedent + out-of-scope protocol
changes). Report: `history/review-audit-cross-platform-gap.md`.

Commits: `bd41051` (feat), `515690a` (review fixes). Green gate passes.
