---
name: shipshape-release
description: >-
  Drive a repository to OSS-release quality and cut releases from it. The
  orchestrator/router of the /shipshape-* family: reads the OSS-RELEASE.md contract,
  scores readiness, sequences the member skills, and hands off to the resumable
  release engine. Thin caller of the `shipshape` binary (the binary is the source
  of truth).
cli_version: "{{CLI_VERSION}}"
schema_version: {{SKILL_SCHEMA_VERSION}}
---

# /shipshape-release

You are taking a repository to open-source release quality, or cutting a release
from one that is already there. The `shipshape` binary does everything that can be
decided deterministically: it normalizes the `OSS-RELEASE.md` contract, detects repo
facts, scores readiness, seals release plans, and runs the resumable release engine.
It never prompts and never guesses a version. What is left for you is judgment: which
kind of work this repository needs right now, in what order to run the member skills,
what version the release should carry, and the conversation with the user at the one
point where something irreversible is about to happen.

This skill was rendered from `shipshape` **{{CLI_VERSION}}**. The binary and its skills
ship as one unit, so if `shipshape version --json` reports a different version, print
the matching skill with `shipshape skill print shipshape-release` and follow that copy
instead. The lockstep between skill and binary is what lets you trust every command and
flag named here.

## The contract comes first

Everything in the family reads the release contract through the normalizer, never from
the raw `OSS-RELEASE.md` prose, because the normalizer materializes defaults and
enforces floors that the prose does not show:

```bash
shipshape contract show --json
```

The JSON error envelope's `error.code` tells the situations apart, and they lead to
different places:

- `contract_not_found`: there is no contract yet. `/shipshape-init` writes a
  reviewable draft. A draft lands as `status: draft` and only a human flips it to
  `approved`; nothing that writes files or publishes should run from a draft, so a
  fresh init is where this invocation ends.
- `invalid_contract`: the contract exists but would not normalize, including a
  `schema_version` newer than this binary knows. Show the diagnostics and stop. A
  contract someone approved is theirs; rewriting, downgrading, or re-initializing it
  to make it validate would discard their decisions.
- No git repository at all (not an error code of `contract show`; `shipshape facts
  --json` reports `is_git: false`): this family adds a public release face to a
  repository that already exists. It never runs `git init`, creates a GitHub
  repository, or bootstraps issue tracking. Point the user at their project-creation
  flow.

`--require-approved` makes `contract show` fail with `not_approved` on a draft. The
mutating members and `release cut` enforce this themselves, so you do not need to
re-check it before each one, but it is a quick way to learn early that a cut is not
yet authorized.

## Which work this repository needs

One invocation either closes readiness gaps or cuts a release. The readiness audit is
read-only and tells you which:

```bash
shipshape audit --json
```

`core_complete` reports the tier-scaled gated core: README and LICENSE, plus CI at
`mvp` and above (a spike is not being published, so CI is only a recommended gap
there). `gaps` lists every unmet obligation, core first. If the user asked for a
release and the core is complete, cut. If the core is incomplete, bootstrap, and say
why: releasing a repository without a README, a license, or CI puts something in
public that nobody can use, license, or trust. If the core is complete and there was no
release intent, offer to close the recommended gaps. An explicit request such as "make
it publishable" or "ship 1.2.0" settles the mode, but it does not remove the
preconditions of that mode; "ship it" on an incomplete core still means explaining the
gap rather than cutting.

A `core_complete` of `unknown` cannot occur today (every core leg is a filesystem
probe), but the audit's GitHub-API checks do degrade to `unknown` when a lookup fails.
Never read an outage as absence; when the report cannot tell you, ask the user which
work they want, in plain conversational text.

Release intent authorizes entering release mode. It does not authorize publishing;
that decision is made separately, below, with the sealed plan in front of the user.

Members never invoke this skill back, so sequencing always has one owner.

## Bootstrap: sequencing the members

`shipshape facts --json` and the audit give you the ecosystems, packages, existing CI,
tags, and the gap list. Present the gaps to the user, distinguishing the core (needed
before any release) from the recommended set (offered, scaled to the contract's
`maturity` tier), and agree on what to close before any file is written. Each member
then produces one reviewable diff.

The order carries one dependency worth knowing: `/shipshape-ci` reports the workflow
file and badge URL it created, and `/shipshape-readme` uses them for the badge row.
Standalone, readme falls back to scanning `.github/workflows/ci*.yml` and, failing
that, emits a placeholder the user must confirm, so running CI first means the README
is written once and correctly. After the core (`/shipshape-ci`, then
`/shipshape-readme` for README and LICENSE) come `/shipshape-changelog`,
`/shipshape-contributing`, `/shipshape-security-policy`, and `/shipshape-architecture`
when the contract's `docs_site` is not `none` or the user asked for architecture docs.
`/shipshape-readme` expects the orchestrator to run `agentify` once at the end of a
bootstrap so the AI-facing `AGENTS.md` matches what was generated.

Each member re-reads the contract and validates its own inputs; you sequence them, you
do not stand in for their checks. If a member fails or the user rejects its diff, stop
and report rather than running a member that depends on the rejected output. Finish
with a fresh audit so the report of what landed and what remains is the binary's, not
your recollection.

### Binary distribution infrastructure

A repository that ships prebuilt binaries needs cargo-dist configuration
(`dist-workspace.toml`) and the tag-triggered `release.yml` cargo-dist generates from
it. `/shipshape-dist` owns that infrastructure and wraps `shipshape dist generate`,
which maps the contract's `distribution` block straight into cargo-dist's config and
then runs `dist generate` for the workflow. The one family decision to carry in your
head: cross-platform is a hard requirement. The normalizer's default platform set is
macOS arm64 plus statically linked musl Linux arm64 and x86_64, and a release path that
builds for one OS is an incomplete release, not a valid one. Intel macOS and Windows
are deliberately not maintained prebuilt channels. `release.yml` belongs to cargo-dist
and to the release cut, never to `/shipshape-ci`, whose files are `ci*.yml`.

When cutting from a repository whose dist infrastructure exists, or whose contract's
`distribution` block names a tap, but whose `targets` do not declare the matching
target, `release cut` refuses with `undeclared_distribution`. That refusal exists
because the alternative is silently skipping a channel users install from; fix the
contract and re-plan rather than working around it.

## Cutting a release

The engine is a journaled, resumable state machine. It knows how to publish; it does
not know what version this release is, and it will not ask. Those are yours.

### Before planning

Check `shipshape release list --json` for an in-flight run. Two cuts cannot run at
once (a `cut_in_progress` error means the single-active-cut lock is held), and an
interrupted run should be reconciled with `resume` or `verify`, not overtaken by a
second plan.

Read the `[Unreleased]` block of `CHANGELOG.md` yourself before planning. A parallel
merge once landed a correct-looking entry in an already-published version's section,
which would have shipped wrong notes and rewritten a version's history; no automated
check catches that. Confirm CI is green on the default branch too: a green local gate
runs on one host, and the tag is about to trigger release workflows on others.

Have the user commit or stash uncommitted work. The engine executes in a clean checkout
of the sealed commit, so stray changes are not swept in, but a tree that differs from
HEAD means the user is looking at something other than what will ship. Do not stash or
discard on their behalf.

### Deciding the version

`release plan --bump major|minor|patch` lets the engine own the bump: it computes the
new version from the manifest, and the cut then sets the workspace version, rewrites
intra-workspace `=` pins, refreshes `Cargo.lock`, finalizes the CHANGELOG (the
`[Unreleased]` section becomes a dated release section, for `curated` and `fragment`
modes; the cut fails before publishing if that section is missing), runs any declared
`bump_hook`, commits, and tags. The arithmetic is strict semver on `X.Y.Z`; a
pre-release or build-metadata version is refused rather than guessed. Choosing the
level is your judgment:

- `conventional_commits: true`: derive it from the commits since the last release tag
  (`tags` in the facts; `git log <LAST_TAG>..HEAD --oneline`). `feat` is minor, `fix`
  is patch, `!` or `BREAKING CHANGE` is major. Show the derivation ("minor: 4 feat, 2
  fix since v1.2.0") so a mislabeled commit can be caught. If nothing since the tag is
  releasable, say so instead of inventing a bump.
- `conventional_commits: false`: the user knows what this release means; ask them for
  the level, offering the log as a non-binding hint.
- `versioning: zerover` keeps the major at 0, so a breaking change is `--bump minor`.
  The engine applies the level literally and does not know the scheme.
- `versioning: calver`, or any version the engine's arithmetic cannot produce: bump the
  manifests and finalize the CHANGELOG in a release commit yourself, then plan without
  `--bump`. Without a bump the plan publishes the version already in the manifest and
  touches no files, so nothing is finalized for you.

`release plan` derives the current version from the workspace manifest alone. If the
manifests disagree (`version_inconsistent_tree`), carry no version
(`version_undeterminable`), or a manifest-versioned target's version cannot be read
(`version_source_unreadable`), it refuses; fix the manifests rather than looking for a
flag. There is no `--version` flag by design: two sources of truth for the version was
a drift footgun.

Package names and product names can differ deliberately: the Cargo package
`shipshape-cli` installs the command `shipshape`. The registry gets the package name,
Release assets and Homebrew formulas the product name; renaming one coordinate never
renames the others.

### Sealing and approving

```bash
shipshape release plan --json --bump <LEVEL>
```

Planning is read-only. It seals a content-addressed plan over the contract, the facts,
and HEAD, persists it in the durable plan store, and exits. Read the warnings: they
name a tag-only plan (no targets), an unresolved package that makes the plan uncuttable,
a dependency between an engine-published crate and a CI-delegated one (uncuttable by
construction, since publish runs before the tag that wakes CI), a missing tag trigger
for a delegated crates.io publisher, the bump's full edit set, and any `bump_hook`
verbatim. That hook is arbitrary code the cut runs with `sh -c` in the release
environment, possibly with publish credentials; the approver must see it.

Now the one conversation that matters. Publishing is irreversible: a crates.io or PyPI
version can never be reused, and a pushed tag on a half-published release is the worst
state a repository can be in. Show the user the `plan_id`, the version, the tag, every
publish destination, the changelog change, and the warnings, and get their explicit
confirmation of this exact plan. What you are protecting is their name on a permanent
public artifact. A repository whose own policy grants autonomous cuts (its `AGENTS.md`
says so, as shipshape's own does) has already given that confirmation in advance;
report what the plan contains and proceed. Anything else earns the question.

### Cutting

```bash
shipshape release cut --plan <PLAN_ID> --json
```

The cut refuses (`not_approved`) unless the contract is `approved`, recovers the bump
disposition from the stored plan (passing `--bump` is optional and must agree),
re-derives the plan from the current tree and refuses if anything hashed into it
changed, validates the host toolchain, and only then creates the run. An invalidated
plan never started anything; re-plan and show the new plan to the user. With
`--json` the cut streams one JSONL event per journaled fact; the first event carries
the `run_id`, and every reconciliation command needs it.

Phases run in a fixed order for a reason: bump, dry-run every target, build every
target, publish every target (crates.io in dependency order, waiting for the index),
tag (a GitHub Release is delegated to cargo-dist CI), dist (the engine writes a
Homebrew formula where it owns the tap), verify, advance-branch. Nothing is tagged
until every publish has succeeded. Verify observes each destination (registry index,
Release assets, tap formula); a target that CI publishes is watched with a bounded
wait, around twenty minutes, because a destination that cannot be observed is not
green. Advance-branch fast-forwards the remote default branch to the release commit
and never force-pushes; if the branch diverged or the push was refused, the run stays
resumable at that phase and the fix is to resolve the cause and resume, not to push
or retag by hand.

### When something goes wrong

There is no automatic rollback of an irreversible step, and there should not be:
the release journal in the repository's git common directory records exactly what
landed, and the remote is ground truth. Inspect it through `shipshape release show`
rather than assuming a worktree-local path.

```bash
shipshape release list --json
shipshape release show <RUN_ID> --json
shipshape release verify <RUN_ID> --json
shipshape release resume <RUN_ID> --json
shipshape release abandon <RUN_ID> --json
```

`verify` is a read-only reconcile against the registries. `resume` reconciles and
continues from the journal, executing the stored plan against a clean checkout of the
sealed commit, so it survives a code fix moving HEAD. It refuses on a target whose
remote state is `unknown`; `--allow-unverified` trusts the journal for such targets
only, never for one observed `missing` or conflicting. `abandon` ends a run that should
not finish (or discards an unused plan by its id) and can break a provably dead
holder's lock. `sealed_commit_unavailable` means the sealed commit is not present in
the clone where the command runs (never committed, not fetched, or garbage-collected).

Never re-publish by hand what the engine was publishing; the journal would then
disagree with the registry and every later reconcile would be wrong. Report the
precise state: which targets are at their destinations, which are not, and what the
user's choices are (resume, verify, or complete a target out of band and accept the
skew). A half-published release presented as either done or failed costs the user the
information they need most.

## What done looks like

A bootstrap is done when a fresh `shipshape audit --json` reports the core complete
and the user has seen what remains. A cut is done when `shipshape release show` reports
the run terminal and complete: every target observed at its destination, the tag
pushed, and the remote default branch containing the release commit. Throughout,
`shipshape contract validate --json` exits zero, because nothing in this flow edits
the contract.
