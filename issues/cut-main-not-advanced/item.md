---
created: 2026-08-23
updated: 2026-08-23
type: bug
reporter: jari
status: open
priority: normal
---

# release cut leaves the bump commit tag-only; main stays at the pre-release version

## Description

On the project-canon v0.6.2 cut (ossctl 0.10.0), the engine created the bump commit (version + pin rewrites + Cargo.lock + changelog), tagged it v0.6.2 and pushed the tag — but never advanced main: origin/main stayed at the pre-bump commit with version 0.6.1 while crates.io/Homebrew shipped 0.6.2. Consequences until manually fixed: a next 'release plan --bump patch' from main would compute 0.6.2 again (collision with the published version), and the finalized CHANGELOG existed only behind the tag. Manual fix was a plain fast-forward (git merge --ff-only <tag-commit> + push), so either the cut should push main itself (ff-only, never force) or the docs/skill must state that merging the release commit back is an operator step. If tag-only is by design, say so in 'release cut --help' and the /oss-release-cut skill.
