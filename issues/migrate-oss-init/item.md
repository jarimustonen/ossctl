---
created: 2026-07-25
updated: 2026-07-26
type: task
status: in-progress
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@skill-subcommand', '@contract-command', '@facts-command']
commits:
- hash: f8eb2fd
  summary: bundle oss-init as skill template shelling out to ossctl facts + contract validate
---

# Migrate /oss-init from homebase into ossctl; delete its Python scripts

## Description

Relocate the already-built /oss-init skill (SKILL.md + SCHEMA.md, currently in homebase dotfiles/src/.claude/skills/oss-init/) into crates/ossctl-cli/skills/oss-init/, with cli_version/schema_version frontmatter. Its two Python scripts are DELETED in favor of the binary subcommands: the skill shells out to 'ossctl facts' + 'ossctl contract validate' instead of bundling check-oss-release.py/infer-repo-facts.py. Remove the homebase copy after landing here (and drop the homebase tw/skill wiring). Blocked by contract-command + facts-command + skill-subcommand.

## Decisions

### 2026-07-25T11:59:35Z · @jari

Delivery model (confirmed by Jari 2026-07-25): family skills do NOT live in homebase dotfiles. Once ossctl ships them they install via 'ossctl skill install' (§15-17, version-pinned, bundled under crates/ossctl-cli/skills/). This issue removes /oss-init's homebase copy; the broader rule — NO family skill stays in homebase, all via 'ossctl skill install' — applies to prose-skills too. Commits/history in homebase may stay; only the live skill files + Python scripts are deleted.
