---
name: oss-release
description: >-
  Drive a repository to OSS-release quality and cut releases from it. The
  orchestrator/router of the /oss-* family: reads the OSS-RELEASE.md contract,
  scores readiness, sequences the member skills, and hands off to the resumable
  release engine. Thin caller of the `ossctl` binary (the binary is the source
  of truth).
cli_version: "{{CLI_VERSION}}"
schema_version: {{SKILL_SCHEMA_VERSION}}
---

# /oss-release

Orchestrator for taking any repository to open-source release quality and
cutting a release from it. This skill is a **thin caller** of the `ossctl`
binary: every deterministic decision (contract normalization, fact detection,
readiness scoring, release mechanics) is delegated to `ossctl`, and this skill
owns only sequencing, user conversation, and the SemVer-bump judgment the
binary deliberately refuses to make.

> **Binary is the source of truth (§17).** This skill was authored against
> `ossctl` **{{CLI_VERSION}}**. If `ossctl version --json` reports a different
> `version`, re-run `ossctl skill print oss-release` to get the skill that
> ships with the running binary before following these steps.

## First act: read the contract

Every reader's first act is to normalize the release contract and gate on the
exit code — never re-derive a default from the raw `OSS-RELEASE.md` prose:

```bash
ossctl contract show --json || exit   # abort on any non-zero exit
```

For a mutating member, require an approved contract:

```bash
ossctl contract show --json --require-approved || exit
```

## Sequence

1. **Detect facts.** `ossctl facts --json` — ecosystems, packages, CI, tags.
2. **Score readiness.** `ossctl audit --json` — the gap report (read-only).
3. **Fill the gaps.** Sequence the member skills for each gap the audit
   reports (README/LICENSE, CI, CHANGELOG, CONTRIBUTING, SECURITY, …).
4. **Plan the release.** `ossctl release plan --json` computes and seals a
   content-addressed plan; the binary exits at the approval boundary rather
   than prompting.
5. **Approve and cut.** Re-invoke `ossctl release cut --plan <plan_id>` to
   execute the sealed plan. `ossctl release resume <run_id>` continues an
   interrupted run; `ossctl release verify <run_id>` reconciles against the
   registry.

## Success criteria

- `ossctl audit --json` reports no blocking gaps.
- `ossctl contract validate` exits 0.
- The release run reaches a terminal `published` + `tagged` state
  (`ossctl release show <run_id> --json`).

> This is the founding mechanism template. The full workflow prose lands with
> the `migrate-oss-init` and `prose-skills` units; the binary command surface
> it references is already the contract.
