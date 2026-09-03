---
created: 2026-08-23
updated: 2026-09-03
type: feature
reporter: jari
status: in-progress
priority: normal
labels: [skills]
lane: publicize
commits:
- hash: 881b7fab459c7ccb17c4410e444d3fff5ddeea37
  summary: start implementation
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

## Comments

### 2026-08-24T09:58:40Z · @glasspad-orchestrator

**Second data point: glasspad publicize pass, 2026-08-24** (run by the glasspad orchestrator session; ended in the 0.17.1 release, verified on all three channels).

What was run: repo-metadata apply (`gh repo edit` description + 9 topics, PVR enable via `gh api PUT`), GitHub issue forms (via `/shipshape-contributing`), `ARCHITECTURE.md` (via `/shipshape-architecture`), a README overhaul as an autonomous worktree unit (logo, live-space screenshot, security-model section, docs links, deep-dive content moved to `docs/`), then full green gate + a docs-only patch cut.

**Confirmations of the gaps already listed (now 2/2 projects):**

- *Repo metadata empty despite an otherwise-complete repo* — description, topics, and homepage were all null on a repo whose `shipshape audit` said `core_complete`. Applied directly under standing autonomy; supports the "apply on explicit go" step.
- *PVR disabled while SECURITY.md points at it* — exactly the project-canon finding, independently rediscovered. One `gh api PUT` fixes it. Nuance worth encoding in the binary: `shipshape audit`'s community profile reported `security: absent` even though a root `SECURITY.md` was committed and pushed — the actionable signal was the PVR **setting**, not file presence. An audit dimension "PVR enabled when SECURITY.md references it" would have caught this precisely; file-presence probing did not.
- *Claims-vs-binary drift is the biggest README rot* — the stealth-era README documented a CLI surface (`serve`, `create`, `render`, `open`, `publish-space`) that no longer exists; the shipped surface is a `publish` default verb + a `loopback` group. Caught only because the README worker re-derived examples and the orchestrator verified them against `--help`. Note for the C-option automation: a canon-conformant CLI exposes `--help --json`, so "README example commands ⊆ real command tree" is mechanizable as an audit dimension, not just skill judgment.
- *AI-face staleness* — confirmed here too: `AGENTS.md` still documents `glasspad serve` post-rename. (Fixed for the observed instances; a full re-ground remains open on the glasspad side.)

**New observations glasspad adds:**

- *Issue-forms vs issuectl tension in `/shipshape-contributing`.* The skill's tracker-detection rule says "issuectl present → do NOT generate GitHub forms". The publicize goal wants the opposite: GitHub Issues as the **external intake** (and the community-profile checkbox), issuectl as the committer-only tracker of record, with an explicit mirror note in the forms. That is the same owner correction (a) recorded from project-canon — with two data points, "external → GitHub Issues; issuectl = committer-only; forms state the mirror" deserves to be the *default* in an external-audience/publicize mode rather than an owner override each time.
- *Registry front page is part of the pass.* crates.io renders the README too: images must use absolute `raw.githubusercontent.com` URLs (relative paths break off-GitHub), and the new README does not actually reach crates.io until a release is cut. A publicize pass on a published crate therefore naturally *ends in a patch release* — "the README isn't shipped until you cut" is a sequencing rule the skill should own.
- *Visual front door needs a fallback ladder.* For a visual tool the screenshot was the single highest-leverage README change, but capture automation is environment-fragile (Apple Events automation dead → headless-browser fallback succeeded). If the skill briefs a worker to capture screenshots, mark the step optional-with-disclosure, never a hard done-criterion.
- *`/shipshape-architecture` fits the pass.* Contributor-facing depth (a code map) slotted naturally between contributing docs and the README rewrite; worth listing as an optional step in the sequence even though it is never a gate.
- *Social preview image is the one metadata item with no CLI/API path* — `gh repo edit` covers description/topics but the social preview requires the web UI; the skill can only surface it as a manual TODO for the maintainer.

**On the design question:** this run leans **A** (thin `/shipshape-publicize` member) with the deterministic checks pushed into `shipshape audit` dimensions: (1) PVR-enabled-when-SECURITY.md-references-it, (2) README-example-commands ⊆ `--help --json` command tree, (3) README install/platform claims vs dist target list, (4) repo description/topics non-empty. B would overload a release-focused orchestrator with a one-time transition; C loses exactly the checks above that are tool-verifiable. The glasspad sequence that worked: metadata apply → issue forms → architecture (opt) → README rewrite as a worktree unit with the claims audit in its brief → green gate → patch cut.
