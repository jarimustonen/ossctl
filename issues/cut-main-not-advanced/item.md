---
created: 2026-08-23
updated: 2026-08-23
type: bug
reporter: jari
status: fixed
priority: normal
lane: release-engine
closed: 2026-08-23
commits:
- hash: 91e8602
  summary: advance-default-branch-after-verified-cut
---

# release cut leaves the bump commit tag-only; main stays at the pre-release version

## Description

On the project-canon v0.6.2 cut (ossctl 0.10.0), the engine created the bump commit (version + pin rewrites + Cargo.lock + changelog), tagged it v0.6.2 and pushed the tag — but never advanced main: origin/main stayed at the pre-bump commit with version 0.6.1 while crates.io/Homebrew shipped 0.6.2. Consequences until manually fixed: a next 'release plan --bump patch' from main would compute 0.6.2 again (collision with the published version), and the finalized CHANGELOG existed only behind the tag. Manual fix was a plain fast-forward (git merge --ff-only <tag-commit> + push), so either the cut should push main itself (ff-only, never force) or the docs/skill must state that merging the release commit back is an operator step. If tag-only is by design, say so in 'release cut --help' and the /oss-release-cut skill.

## Resolution

### 2026-08-23T13:42:34Z · @issuectl

Delivered journal schema v6 and SEAL_VERSION 10 final advance-branch barrier. The engine durably selects origin's default branch, fast-forwards without force after destination verification, preserves legacy run semantics, and resumes safely after divergence, permission, or journal failures. Full green gate and multi-model review/assessment passed.
