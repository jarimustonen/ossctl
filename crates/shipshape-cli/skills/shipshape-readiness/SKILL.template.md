---
name: shipshape-readiness
description: >-
  Score how far a repository is from a responsible open-source release and turn the
  result into an ordered plan of which /shipshape-* member closes what, and why in that
  order. A read-only, thin caller of `shipshape audit`: the binary scores the gated
  core (README, LICENSE, CI at mvp and above), the maturity-scaled recommended set,
  the producer obligations the OSS-RELEASE.md contract implies, the deterministic
  public-front-door checks, and GitHub's community profile; this skill interprets the
  report, separates blocking from recommended from could-not-check, and sequences the
  fixes. Writes nothing and runs no member. Use for "how ready is this repo", "audit
  release readiness", "what is missing before we can release", or "score this
  repository". Not for closing the gaps (/shipshape-release sequences the members) or
  for writing the contract (/shipshape-init).
allowed-tools: Bash, Glob, Grep, Read
cli_version: "{{CLI_VERSION}}"
schema_version: {{SKILL_SCHEMA_VERSION}}
---

# /shipshape-readiness

You are answering one question for a maintainer: how far is this repository from a
public release they could stand behind, and what should happen next, in what order.
The `shipshape` binary does the scoring. It is deterministic, read-only, and the same
code every family member reads, so its verdict on the gated core is the family's
verdict. What it cannot do is explain the report to a person, tell a gap that is work
from a gap that was decided against, notice that a contract dial rather than a file is
what is wrong, or put the fixes in an order that means each member runs once. That is
your part. This skill changes nothing in the repository and publishes nothing; its
deliverable is the reading of the report and the plan.

This skill was rendered from `shipshape` **{{CLI_VERSION}}**. The binary and its skills
ship as one unit, so if `shipshape version --json` reports a different version, print
the matching skill with `shipshape skill print shipshape-readiness` and follow that
copy. When this text and the binary disagree, the binary is right.

Arguments: `$ARGUMENTS`. A positional path names the target repository (default: the
current directory's repository). Resolve it to the git root and pass that root to every
`shipshape` call, because the binary looks for `OSS-RELEASE.md` exactly where it is
pointed and does not walk up from the process cwd.

## Where the facts come from

```bash
shipshape audit --json --repo-root <REPO_ROOT>
```

That one command is the audit. It runs the same normalizer as `contract show` and the
same detector as `facts` before scoring, so there is no separate gate to pass first: a
missing contract fails with `contract_not_found` and an invalid one with
`invalid_contract` and the list of problems, exit 1 either way. The fix for the first is
`/shipshape-init`; the second is a contract someone wrote, so show the problems rather
than rewriting it. Approval is not required. The audit reads a `status: draft` contract
on purpose, which is why `/shipshape-init` hands off here straight after writing one.
Every member that writes files refuses a draft, though, so a plan built from a draft
starts with the human review that sets `status: approved`, and says so.

Two things about the exit status that the help text does not say. Gaps are data, not
errors: a repository with an incomplete core exits 0, so the status tells you the audit
ran, never that the repository is ready. And the audit makes read-only network calls
(`git remote get-url origin`, then `gh api` for the community profile, the repository
metadata, and the vulnerability-reporting setting), which need no one's permission but
do take a few seconds and can fail; see the `unknown` discipline below.

When you need to explain a gap in the contract's own terms, or to judge whether the
maturity tier fits the project, these give you the normalized dials and the detected
facts the audit scored against:

```bash
shipshape contract show --json --repo-root <REPO_ROOT>
shipshape facts --json --repo-root <REPO_ROOT>
```

Read the contract through the normalizer and never from the raw prose; the normalizer
materializes defaults (the cross-platform platform set, for one) that the prose does
not show.

## Reading the report

`data` carries `maturity`, `core_complete`, `gaps`, and `community_profile`. Gaps are
listed in a stable order, core first, then the recommended set, then producer
obligations, then the front-door checks, and only unmet obligations appear; a satisfied
check produces nothing. Key off `id`, which is stable, not `detail`, which is prose for
a person. Each gap also names the `member` skill that closes it.

**The gated core** is what `core_complete` reports, and only core gaps carry
`severity: blocking`. It is README and LICENSE, plus CI at `mvp` and above. A `spike`
is not being published, so CI is reported there as a recommended gap toward `mvp`
rather than a core failure. README and LICENSE are found under the common spellings
(`README.md`, `LICENSE`, `COPYING`, and the like) at the root, `.github/`, or `docs/`.
CI means the facts detector found one of `.github/workflows` with at least one entry,
`.gitlab-ci.yml`, `.circleci`, `azure-pipelines.yml`, `.drone.yml`, or a `Jenkinsfile`.
A project whose CI lives somewhere else gets a `ci` core gap that is wrong, and the
honest reading is to say the detector did not see it, not to propose a second CI.
`core_complete` is `complete` or `incomplete`; the `unknown` value is reserved and
cannot occur today, since every core leg is a filesystem probe.

**The recommended set** (`category: canon`) is cumulative by tier. `mvp` adds a
changelog, a contributing guide, a code of conduct, a security policy, and a dependency
bot (`.github/dependabot.yml` or a Renovate config). `production` adds CODEOWNERS,
GOVERNANCE, ARCHITECTURE, a pre-commit config, and a coverage step in CI. None of these
blocks a release. If the list of production-tier gaps looks absurd for the project in
front of you, the maturity dial is probably wrong, and that is a contract question for
the maintainer and `/shipshape-init`, not a stack of work to schedule.

**Producer obligations** (`category: producer`) are gaps the contract created for
itself: it declared something whose producer is not there. A `fragment` changelog mode
without its fragment directory; a `coverage`, `scorecard`, `ci`, or `license` health
badge without the CI step or file that would make it true; a declared binary
distribution whose `platforms` omit Linux or macOS (`distribution-linux`,
`distribution-macos`, suffixed with the package name in a monorepo). Each has two
honest fixes, create the producer or change the dial, and the report should offer both
rather than assume the file is wanted. One of them bites harder than its `recommended`
severity suggests: the family's cross-platform decision (macOS arm64 plus musl Linux
arm64 and x86_64 for every prebuilt channel) means a one-OS distribution is a release
gap, not a variant. The normalizer defaults an omitted `platforms` to the full set and
rejects an empty one, so a missing OS is always an explicit authoring choice worth
naming as such.

**The front-door checks** carry `member: shipshape-publicize` (their `category` is
`canon` too, so the member, not the category, is what separates them from the
recommended set): empty GitHub description or topics, a security policy that promises
Private Vulnerability Reporting on a repository where it is not enabled, README claims
of prebuilt binaries for platforms no distribution builds, README command examples that
the project's own binary does not accept, links to tracked symlinks that GitHub renders
as a path, and "Claude Code skill" where the category term is Agent Skills. They
describe whether a stranger can use the project, not whether it can be released, and
they are best closed as one pass by `/shipshape-publicize` rather than picked off one by
one. One of them needs a caveat in your report: the README command check compares the
binary on `PATH` against a clean `HEAD`, so right after a README or CLI change it reads
`unknown` until the tree is committed and the binary rebuilt. That is expected, not a
finding.

**The `unknown` discipline.** Every check distinguishes checked-and-absent from
could-not-check. Filesystem probes are always determinate. A GitHub lookup that fails,
a `gh` that is missing, a remote that is not GitHub, or a workflow file that could not
be read yields `status: unknown`, never `absent`, because an outage read as a missing
artifact would send someone to create a file that exists. `unknown` is not a gap to
close and it is not green; it goes in its own section of the plan as something to
re-check or to disclose. The `coverage` and `scorecard` producer checks are substring
scans of the workflow YAML, so a comment that mentions coverage satisfies them and an
unusual tool does not; when a result there contradicts what you can see in the
workflows, say what you saw.

**`community_profile`** is GitHub's own view of the health files, from the repository's
community-standards endpoint. `checked: false` with a reason means the lookup could not
run (no GitHub remote, no `gh`, a private or absent repository), and every field is
`unknown`. When it did run it is corroborating evidence: `present` on GitHub for a file
the local probe also found is confirmation, and `absent` on GitHub for a file that is
present locally usually means the work has not been pushed yet, which belongs in the
report.

## Deciding what is actually work

A gap is work only if the maintainer wants the artifact. Repositories record decisions
against some of them: a project that has decided it does not want a code of conduct, or
a governance file, will say so in its `AGENTS.md`, its contributor docs, or its issue
tracker. Look before you schedule. A recorded decision turns the gap into an accepted
one that the report lists as such and does not argue with; the audit will keep
reporting it, which is expected. Do not invent such a decision from silence, either.
When nothing is recorded, the gap is an offer for the maintainer to accept or decline,
and the report presents it that way.

## Putting the fixes in order

The order comes from the dependencies between members and from what is at stake, not
from the severity label alone.

Blocking gaps first, because nothing publishes responsibly without them. Within the
core, `/shipshape-ci` before `/shipshape-readme`: the CI member reports the workflow
file and badge URL it created, and the README member uses them for its badge row.
Standalone, the README member falls back to scanning `.github/workflows` and, failing
that, leaves a placeholder the user has to confirm. Running CI first means the README
is written once. README and LICENSE are one `/shipshape-readme` run.

Producer gaps next, resolved in the direction the maintainer chooses. Where the fix is
a contract change, it is one edit and a re-approval before any member runs, because
every member re-reads the contract and a member run against the old dial produces the
old result.

Then the recommended set, grouped by member so each runs once: `/shipshape-contributing`
covers contributing, code of conduct, CODEOWNERS, and governance in one run;
`/shipshape-ci` covers the dependency bot, pre-commit, and the coverage step alongside
the workflow; `/shipshape-changelog`, `/shipshape-security-policy`, and
`/shipshape-architecture` each own one. `/shipshape-contributing` links to a
`SECURITY.md`, so the security policy follows it. Architecture docs are never required
of any project, so they go last and are marked optional.

Front-door gaps as one `/shipshape-publicize` pass, after the artifacts it would speak
about exist. Unknowns in a section of their own.

Members never invoke each other or the orchestrator, so sequencing always has one
owner. This skill produces the plan; executing it belongs to `/shipshape-release`, which
runs the members in order and finishes with a fresh audit, or to the individual member
when the user asks for one gap by name. If you are asked to close the gaps yourself,
hand the plan to `/shipshape-release` rather than becoming a second sequencer. Whoever
runs the members, the statement of what remains afterwards should come from a fresh
`shipshape audit --json`, not from memory of what was generated; the audit is cheap and
its report is the binary's.

## Reporting

Lead with the verdict: the maturity tier the scoring assumed, whether the core is
complete, and whether the contract is approved. Then the plan, in the order above, with
one line per gap naming the member and the reason it sits where it does; producer gaps
with both of their fixes; accepted gaps with where the decision is recorded; unknowns
with what could not be checked and why; and the one thing to run next, whether that is
approving the contract, `/shipshape-release`, or a single member. Keep the audit's
`detail` text out of the report except where it says something the maintainer needs;
they can read the JSON, and the value you add is the ordering and the reasons. Then
stop.
