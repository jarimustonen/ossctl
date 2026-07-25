---
created: 2026-07-25
updated: 2026-07-25
type: task
status: open
priority: normal
epic: ossctl-phase4-build
blocked_by: ['@skill-subcommand', '@contract-command', '@facts-command']
---

# Migrate /oss-init from homebase into ossctl; delete its Python scripts

## Description

Relocate the already-built /oss-init skill (SKILL.md + SCHEMA.md, currently in homebase dotfiles/src/.claude/skills/oss-init/) into crates/ossctl-cli/skills/oss-init/, with cli_version/schema_version frontmatter. Its two Python scripts are DELETED in favor of the binary subcommands: the skill shells out to 'ossctl facts' + 'ossctl contract validate' instead of bundling check-oss-release.py/infer-repo-facts.py. Remove the homebase copy after landing here (and drop the homebase tw/skill wiring). Blocked by contract-command + facts-command + skill-subcommand.
