---
created: 2026-07-25
updated: 2026-07-26
type: feature
status: in-progress
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@workspace-scaffold']
commits:
- hash: 4f7e7a687ea7
  summary: 'feat(skill): skill list|install|print + bundle mechanism + §17 lockstep gate'
---

# ossctl skill list|install|print + bundle the /oss-* skills (§15-17)

## Description

Implement the §15-17 companion-skill installer: 'ossctl skill list|install|print', binary-is-source-of-truth, cli_version/schema_version frontmatter, version-pinned via token substitution at install (issuectl pattern). Bundle all 10 /oss-* skills under crates/ossctl-cli/skills/<name>/SKILL.template.md, installed by 'ossctl skill install' exactly as orchestratectl ships the /worktree-* family. CI lockstep gate (§17): verify every referenced subcommand/flag exists against the --help --json snapshot; golden-test install+print. Skills land here over time (see migrate-oss-init, prose-skills); this issue builds the mechanism + wires the first ones.
