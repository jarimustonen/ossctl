---
created: 2026-09-29
updated: 2026-09-29
type: feature
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-session
source_ref: agent:homebase-session/reporter:jari/id:homebase-agentify-retired-2026-09-29
---

# Remove references to the retired agentify skill from shipshape skills

## Description

Remove references to the retired agentify skill from shipshape skills

Homebase retired its `agentify` skill on 2026-09-29. A trial showed that
`rethink-instructions` covers rewriting and tightening an AGENTS.md, so the
separate skill was removed from `dotfiles/src/.claude/skills/` on all machines.

Three bundled shipshape skills still tell the agent to run it:

- `crates/shipshape-cli/skills/shipshape-release/SKILL.template.md` line 108:
  the orchestrator runs `agentify` once at the end of a bootstrap.
- `crates/shipshape-cli/skills/shipshape-publicize/SKILL.template.md` lines 6
  and 185: the description lists it as a member, and the body hands AGENTS.md
  to it for a re-ground against current source.
- `crates/shipshape-cli/skills/shipshape-readme/SKILL.template.md` lines 12,
  283 and 296: the description routes AGENTS.md to it and the body says
  AGENTS.md is agentify's.

Observed: an agent following these skills is told to invoke a skill that no
longer exists. Expected: the skills say what is wanted in their own words.

What the three call sites need is small and differs from a rewrite: after a
bootstrap, AGENTS.md should carry the same install, usage and release facts as
the README that was just generated; when publicizing, AGENTS.md should be
brought in line with the software as shipped, removing stale "not implemented"
and old-command claims, as a reviewable change and not a replacement file. One
or two sentences in each skill can say this. Where a full rewrite of a grown
AGENTS.md is wanted, point to `rethink-instructions`, and for stale claims to
`fact-check-instructions`.

Until a release carries the change, the stale reference is harmless beyond
the missing skill: the agent has to update AGENTS.md by its own judgment.
