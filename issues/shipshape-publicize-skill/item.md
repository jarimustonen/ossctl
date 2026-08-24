---
created: 2026-08-23
updated: 2026-08-24
type: feature
reporter: jari
status: open
priority: normal
labels: [skills]
---

# Collect de-stealth experience toward a /shipshape-publicize skill

## Description

## Context

On 2026-08-22/23 a full "de-stealth" pass was run manually on **project-canon**
(github.com/jarimustonen/project-canon): the repo had been public-but-unannounced
("stealth public"), and the maintainer wanted it turned into a project that can
attract external users. The session used the existing `/shipshape-*` members
(then named `/oss-*`) plus a set of manual steps no member owns. This issue is the **collection point** for that
experience. Per the maintainer's plan: run the same pass on at least one more
project, append its observations here as comments, and only then extract the
skill — one project is a single data point, not a pattern.

## What the pass actually consisted of (observed sequence)

1. **README rewrite for an external audience** (`/shipshape-readme`, but as a full
   rewrite, not a marker refresh): the stealth-era README assumed family-insider
   context ("the AI-first CLI / project family"); the public one leads with the
   problem the tool solves, a Why section, a worked quickstart from a real run,
   and an honest ZeroVer Status section.
2. **Fact-check every README claim against the running binary and the release
   config** — run `--help` on every verb, compare install/platform claims to
   `dist-workspace.toml` / the contract. This found a real error: the README
   promised macOS x86_64 prebuilt binaries that are not in the dist target list.
3. **AI-face sync (`agentify` on AGENTS.md)** — the agent doc still said
   "bootstrap only, verbs not built yet" four releases after they shipped.
   Stealth repos accumulate stale self-descriptions; the pass must re-ground
   them.
4. **GitHub repo metadata applied, not just suggested** — description and topics
   were both empty; `/shipshape-readme` only prints `gh repo edit` guidance. On the
   maintainer's go the commands were executed.
5. **`/shipshape-contributing`** — worked as designed, with two owner corrections:
   (a) issue-channel split had to be stated explicitly (external contributors →
   GitHub issues; the in-repo issuectl tracker is committer-only), and (b) the
   tier-default CODE_OF_CONDUCT.md was removed on maintainer decision — tier
   defaults are proposals, not mandates.
6. **`/shipshape-security-policy`** — threat gate fired (subprocess probes, untrusted
   probe-output parsing, shipped binaries) → full mvp-scale policy. The skill's
   PVR guidance was not enough: Private Vulnerability Reporting was *disabled*,
   so the policy pointed at a dead button until the maintainer said "aja" and it
   was enabled via `gh api`.
7. **Symlink-rendering fix** — GitHub's web UI renders a symlinked file as its
   target path, not its content. README/CONTRIBUTING links had to target the
   physical master (`crates/.../AGENTS-AI-FIRST-CLI.md`), not the repo-root
   symlink. Any repo using the CLAUDE.md→AGENTS.md / packaged-master conventions
   hits this.
8. **Product-name neutralization** — "Claude Code" references replaced across
   docs, shipped `--help` text, doc comments, and the canon itself with the open
   **Agent Skills** standard (agentskills.io) and neutral `--agent` layout
   identifiers (`claude`/`pi`/`codex`). Multi-agent users don't want one
   vendor's product name as the category term. This touched *shipped* content,
   so it needed the green gate and a changelog fragment, and only reaches skill
   consumers at the next release cut.

## Gaps in the current /shipshape-* family this exposed

- No member **applies** repo metadata (description/topics/social preview) —
  `/shipshape-readme` stops at printed guidance. Fine as a default, but the
  publicize pass wants an "apply on explicit go" step.
- No member **verifies PVR is actually enabled** when SECURITY.md points at it
  (`gh api repos/<o>/<r>/private-vulnerability-reporting` is a cheap read).
- No member does the **claims-vs-binary audit** of an existing README (step 2).
  `/shipshape-readme` trusts contract + facts; the drift that matters was between
  *prose claims* and the *dist target list / real `--help` surface*.
- No member owns the **audience reframing** judgment (insider shorthand → value
  proposition) or the **AI-face staleness** re-grounding (steps 1, 3).
- The **symlink-on-GitHub** pitfall and the **product-name neutrality** check
  are encoded nowhere.

## Design question (to settle after the second data point)

Options, with a preliminary lean:

- **A (lean): a new thin member, working name `/shipshape-publicize`** — an
  orchestrator-style checklist that sequences existing members
  (readme → contributing → security-policy → readiness) in "external audience"
  mode and itself owns only the uncovered gaps above (claims audit, metadata
  apply-on-go, PVR check, symlink-link check, neutrality sweep, AI-face
  re-ground handoff). Members stay sole writers of their files.
- **B: a mode of `/shipshape-release`** — the family already has one orchestrator;
  a `--publicize` flavor of its bootstrap run would avoid a new member but
  overloads a release-focused skill with a one-time transition.
- **C: no skill, just a checklist doc** — cheapest, but the pass has real
  tool-verifiable steps (PVR state, dist-target comparison) that deserve
  automation in `shipshape` (e.g. a `shipshape audit` dimension: "readme claims
  match distribution surface", "PVR enabled when SECURITY.md references it").

Whichever wins, the deterministic checks belong in the **binary** (audit
dimensions), the judgment in the skill — same split as the rest of the family.

## Next step

Run the same pass on a second project (any stealth-public family repo),
append the observations as a comment here, then decide A/B/C and extract.
