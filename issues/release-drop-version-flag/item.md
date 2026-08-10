---
created: 2026-08-10
updated: 2026-08-10
type: improvement
status: done
priority: normal
epic: ossctl-phase4-build
related: ['@release-version-single-source']
commits:
- hash: 31b1c2a
  summary: remove --version input; version derived solely from manifest (breaking); all callers updated
- hash: 4ae2645
  summary: fix stale --version docs in cut/resolve_version; assert clap UnknownArgument in harness tests
closed: 2026-08-10
---

# drop --version as a release input entirely; the workspace manifest is the only source of truth

## Description

Finishes release-version-single-source (0.2.5 kept --version as an optional must-match confirmation). Remove --version as a version INPUT so the release version can ONLY come from the workspace manifest — eliminates the two-masters vector at the root. BREAKING CLI change: update every in-repo caller (the AGENTS.md cut recipe, the /oss-* skill templates under crates/ossctl-cli/skills/*, tests) that passes --version. Worker's call on least-breakage path (hard-remove vs accept-but-ignore-with-deprecation-warning), documented. Filed from the cut-noop /llm-review (stint #16).
