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

Orchestrator for taking any repository to open-source release quality and
cutting a release from it. This skill is a **thin caller** of the `shipshape`
binary: every deterministic decision (contract normalization, fact detection,
readiness scoring, release mechanics) is delegated to `shipshape`. This skill owns
only what the binary deliberately refuses to do — **mode selection, member
sequencing, user conversation, and the SemVer-bump judgment** — and renders the
approval boundary the binary stops at.

> **Binary is the source of truth (§17).** This skill was authored against
> `shipshape` **{{CLI_VERSION}}**. If `shipshape version --json` reports a different
> `version`, re-run `shipshape skill print shipshape-release` to get the skill that
> ships with the running binary before following these steps.

## First act: read the contract

Every reader's first act is to normalize the release contract and gate on the
exit code — never re-derive a default from the raw `OSS-RELEASE.md` prose:

```bash
shipshape contract show --json || exit   # abort on any non-zero exit
```

On a non-zero exit, **read the JSON error envelope's `error.code` / `error.message`
to tell the causes apart** — the same `|| exit` covers several, and they route
differently:

- **No `OSS-RELEASE.md` yet?** Do not invent a config — invoke **`/shipshape-init`** to
  generate a reviewable draft, then STOP: the draft lands `status: draft` and a
  human must review and flip it to `status: approved` before any mutating step
  runs on a later invocation.
- **Not a git repository?** This family never bootstraps a repo (no `git init`,
  no GitHub repo, no `issuectl init`). Stop and point the user at
  `create-project`; `/shipshape-release` only adds the public release face to a repo
  that already exists.
- **Contract present but invalid or a newer `schema_version` than this binary
  knows?** Surface the validation diagnostics and stop — never rewrite,
  downgrade, or re-init a contract that merely failed to validate.

Before any **mutating** work (cutting a release, or the bootstrap generators),
require an approved contract — a `draft` config can never authorize a mutation:

```bash
shipshape contract show --json --require-approved || exit
```

## Pick the mode (never a silent guess)

One `/shipshape-release` run is **either** closing readiness gaps **or** cutting a
release — exclusive per invocation. Select the mode this way (design §1.2):

1. **Explicit override wins.** If the user said `--bootstrap` / "make it
   publishable", or `--cut` / "ship 1.2.0" / "publish", honor it and skip
   detection.
2. **Else audit-first.** Run the readiness audit (read-only) and read the gap
   report's core status:

   ```bash
   shipshape audit --json || exit
   ```

3. **Decide:**
   - core **incomplete** → **bootstrap** (you cannot responsibly release a repo
     missing README / LICENSE / CI — say why).
   - core **complete** and the phrasing is release-intent → **cut-release**.
   - core **complete**, no release intent, recommended gaps remain → **bootstrap**
     (offer to close them; never write a file without the checkpoint below).
   - core status **unknown** (an API lookup failed — never read an outage as
     "no prior release"), or complete with no gaps and no intent → **ask** the
     user which they want, in plain conversational text (no option cards).

**Non-recursion.** `/shipshape-release` invokes members; **no member invokes
`/shipshape-release`**. "Release intent" authorizes *entering* cut-release mode — it
never authorizes publishing. Publishing is gated separately at the approval
boundary below.

**Override selects the mode; it never disarms a gate.** `--cut` on a repo with an
incomplete core still fails the core gate below (with a reason) — an explicit
mode choice picks *which* path runs, not *whether* its preconditions hold.

## Bootstrap mode — sequence the members

Detect facts, score the gaps, checkpoint with the user, then sequence the
generators. The order matters — **CI before README** kills the badge cycle
(`/shipshape-ci` returns the workflow name + badge URL that `/shipshape-readme` consumes, so
the README is written exactly once):

```bash
shipshape facts --json || exit    # ecosystems, packages, CI, tags
shipshape audit --json || exit    # the gap report that drives sequencing
```

1. Present the gaps — **core gaps (must-fix) vs. recommended (offered)** — and
   get the user's go-ahead **before any file is written**.
2. Close the core: **`/shipshape-ci`** → **`/shipshape-readme`** (README + LICENSE).
3. Then the recommended set, scaled to the contract's `maturity` tier, serially
   (one reviewable diff at a time): **`/shipshape-changelog`** →
   **`/shipshape-contributing`** → **`/shipshape-security-policy`** →
   **`/shipshape-architecture`** (only when the contract opts in — its `docs_site` is
   not `none`, or the user explicitly asked for architecture docs).
4. Re-audit and report what landed and what remains:

   ```bash
   shipshape audit --json || exit
   ```

**Stop on member failure.** If a member fails, or the user rejects its diff,
**halt the sequence** — do not invoke a dependent member (notably: never run
`/shipshape-readme` if `/shipshape-ci` did not return its workflow name + badge URL, or it
will write a broken badge). Report what changed, re-audit, and wait for
resolution. Each member self-validates its own inputs (it re-runs
`contract show`); the orchestrator sequences them but does not substitute for
their gates.

## Cut-release mode — own the bump, hand off to the engine

The release engine is a **resumable, self-gated state machine** — it never
prompts and never derives the version. This skill supplies the two things the
binary cannot: the **approved SemVer bump** and the **human approval** between
sealing the plan and executing it.

### Release-infra generation — cross-platform by default (Mac + Linux)

The downstream project's binary-release infrastructure — the cargo-dist config
(`dist-workspace.toml`) and the tag-triggered `.github/workflows/release.yml`
generated from it via `dist generate` — MUST be **cross-platform by default**.
This is `/shipshape-*` family canon (see `AGENTS.md` → "Cross-platform is a hard
requirement (macOS AND Linux)"): a release path that builds on only one OS is an
incomplete release, not a valid one. When establishing or refreshing that config,
map the contract's `distribution` block straight through — never invent a
narrower target set:

- **`distribution.platforms` → cargo-dist `[dist] targets`.** The contract's
  platform list is a set of Rust target-triples. Copy it verbatim into `targets`.
  When the contract **omits** `platforms`, the normalizer's cross-platform
  default applies — `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` (macOS arm64 + x86_64
  and **statically-linked musl Linux** arm64 + x86_64). **Never emit a
  macOS-only matrix.** Add `x86_64-pc-windows-msvc` only when the contract lists
  it.
- **`distribution.installers` → cargo-dist `[dist] installers`.** Ensure `shell`
  is present so the generated curl-installer covers **both macOS and Linux** on
  the Unix side; carry `powershell`/`msi` through only when a Windows triple is in
  the target set. A `homebrew` installer requires `distribution.homebrew_tap`
  (the contract already enforces this floor).
- **Mirror `shipshape`'s own `dist-workspace.toml`** (repo root) as the reference
  shape: pinned `cargo-dist-version`, `ci = "github"`, `hosting = "github"`,
  `github-attestations = true`, `pr-run-mode = "skip"` (tag-triggered only).
- **`dist generate` is the sole author of `release.yml`** — edit config in
  `dist-workspace.toml`, then regenerate; never hand-edit the workflow. Owning
  `release.yml` (the tag-triggered publish/build/sign workflow) is the
  release-cut's job, **not** `/shipshape-ci`'s (which owns `ci*.yml`).

> **Generating it.** The engine generates this infra for you: `shipshape dist
> generate` reads the contract's `distribution` block, writes `dist-workspace.toml`
> in the reference shape (the mapping above — cross-platform by default), and then
> invokes `dist generate` to emit `.github/workflows/release.yml` from it (the
> workflow is never hand-authored — cargo-dist is its sole author). It refuses to
> clobber an existing `dist-workspace.toml` without `--force`, and supports
> `--no-workflow` to write only the config when the `dist` tool is unavailable.
> Only the `cargo-dist` distribution adapter is generated today; a
> `goreleaser`/`manual` scaffolder is a follow-up. The Homebrew formula stays with
> shipshape's own tap adapter (post-tag), so `homebrew` is deliberately kept out of
> the generated cargo-dist installer set even when the contract lists it.

**1. Gate.** First, check for an already-active run so two cuts never race — if
one is in flight, reconcile it (`resume` / `verify` below) instead of sealing a
second plan. Then require an approved contract, a complete core, and a clean
working tree:

```bash
shipshape release list --json || exit    # an in-flight run? resume/verify it, don't start a second
shipshape contract show --json --require-approved || exit
shipshape audit --json || exit           # refuse cut-release while the core is incomplete
```

`release plan` / `cut` are content-addressed and **refuse on repo drift**; a
dirty tree also risks sweeping uncommitted work into the release. Have the user
commit or stash first — do not stash or discard changes on their behalf.

**2. Decide the version (this skill's judgment — design §3.4).** `release plan`
derives the version **solely from the workspace manifest** — there is no
`--version` input (`release-drop-version-flag`). So choosing the number is still
your job, but you apply it by **bumping the manifest** (and finalizing the
CHANGELOG) in the release commit *before* planning; the plan then reads it back.
Read the contract's `conventional_commits` and `versioning`. Find the last
release tag from `shipshape facts --json`, then read the commits since it:

```bash
git log <LAST_TAG>..HEAD --oneline    # the commit set the bump is computed from
```

- `conventional_commits: true` → derive the bump from those commit types
  (`feat`→minor, `fix`→patch, `!` or `BREAKING CHANGE`→major; `zerover` keeps
  major at 0, so a breaking change bumps minor; `calver` computes from its
  pattern). **Propose the version and show the derivation** ("1.3.0 — 4×feat,
  2×fix since v1.2.0") so a human can catch a mislabeled commit; if no commit is
  releasable, say so rather than inventing a bump.
- `conventional_commits: false` → **ask the user** for the exact version in plain
  text, offering that `git log` as a non-binding hint. Accept a concrete version
  matching the contract's `versioning` scheme (not a bare `minor`/`patch`).

Then **bump the manifest to that version** (e.g. `workspace.package.version` and
any internal `=X.Y.Z` dep in lockstep) and finalize the CHANGELOG, in the release
commit. `release plan` reads the version from the manifest, so if the manifests
disagree or carry no version it refuses (`version_inconsistent_tree` /
`version_undeterminable` / `version_source_unreadable`) — relay the error and fix
the manifest rather than re-passing a flag.

Before sealing, preserve each target's authored identity. A Cargo registry package
may deliberately differ from its installed command (for example, package
`shipshape-cli` declares binary `shipshape`). Use the Cargo package for crates.io,
but keep the command/product name for GitHub Release assets and Homebrew formulas;
never infer that renaming one coordinate renames every channel.

**3. Seal the plan.** The binary derives the version from the manifest, computes a
content-addressed plan, and **exits at the approval boundary** rather than
prompting (ADR-0001 §3):

```bash
shipshape release plan --json || exit
```

**4. Render the approval boundary (the one human checkpoint).** Show the user the
sealed `plan_id`, the version, the one shared tag, the changelog diff, and every
publish destination — all from the `release plan --json` payload. Publishing is
**irreversible** (crates.io/PyPI versions are permanent) — state that plainly and
require an explicit confirmation of this exact plan + version before proceeding.
This is the *only* approval prompt; step 2 merely settled the number.

**5. Cut, only after human approval.** Re-invoke with the sealed plan; the cut
re-derives the version from the manifest and refuses if the repo drifted from what
the plan hashed (a manifest-version edit since sealing shows up here). **Capture
the `run_id` from its output** — every reconciliation command below needs it:

```bash
shipshape release cut --plan <PLAN_ID> --json || exit
```

A successful cut does not leave the bump commit reachable only from its tag. After
all publish destinations verify green, the engine resolves `origin`'s advertised
default branch and pushes the release commit to it with an ordinary
fast-forward-only ref update. It never force-pushes and does not depend on the
current checkout being attached to that branch, so sealed resume worktrees behave
the same as the original checkout. A divergent branch, missing default-branch
advertisement, network error, or permission denial leaves the run resumable in the
final `advance_branch` phase. Fix the reported cause and run `release resume`; do
not publish or retag by hand.

On a **drift refusal** the cut never started: have the user reconcile the working
tree, then re-run `release plan` (a new `plan_id`) and re-render the boundary.

**6. Interruptions and reconciliation.** A dropped network / OTP timeout /
one-of-N registry failure is recoverable from the journal — never re-publish by
hand. If the session died before you captured the `run_id`, recover it with
`shipshape release list --json`:

```bash
shipshape release list --json               # find the in-flight run_id
shipshape release show <RUN_ID> --json      # progress (live) or post-mortem
shipshape release resume <RUN_ID> --json    # reconcile + continue from the journal
shipshape release verify <RUN_ID> --json    # read-only reconcile vs. registry state
```

Report full success **or the precise partial state** — never present a
half-published release as wholly done or wholly failed. There is no automatic
rollback of an irreversible step; surface the concrete choices to the human
(resume the pending target, verify against the registry, or accept the skew and
complete the missing publish out of band).

## Success criteria

- **Bootstrap:** `shipshape audit --json` reports no blocking core gaps.
- **Cut-release:** the run reaches terminal `completed` only after publishes and
  destinations verify, the tag lands, and the remote default branch contains the
  release commit, confirmed by `shipshape release show <RUN_ID> --json`.
- The contract validates throughout: `shipshape contract validate --json` exits 0.
