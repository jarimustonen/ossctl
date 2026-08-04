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
readiness scoring, release mechanics) is delegated to `ossctl`. This skill owns
only what the binary deliberately refuses to do — **mode selection, member
sequencing, user conversation, and the SemVer-bump judgment** — and renders the
approval boundary the binary stops at.

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

On a non-zero exit, **read the JSON error envelope's `error.code` / `error.message`
to tell the causes apart** — the same `|| exit` covers several, and they route
differently:

- **No `OSS-RELEASE.md` yet?** Do not invent a config — invoke **`/oss-init`** to
  generate a reviewable draft, then STOP: the draft lands `status: draft` and a
  human must review and flip it to `status: approved` before any mutating step
  runs on a later invocation.
- **Not a git repository?** This family never bootstraps a repo (no `git init`,
  no GitHub repo, no `issuectl init`). Stop and point the user at
  `create-project`; `/oss-release` only adds the public release face to a repo
  that already exists.
- **Contract present but invalid or a newer `schema_version` than this binary
  knows?** Surface the validation diagnostics and stop — never rewrite,
  downgrade, or re-init a contract that merely failed to validate.

Before any **mutating** work (cutting a release, or the bootstrap generators),
require an approved contract — a `draft` config can never authorize a mutation:

```bash
ossctl contract show --json --require-approved || exit
```

## Pick the mode (never a silent guess)

One `/oss-release` run is **either** closing readiness gaps **or** cutting a
release — exclusive per invocation. Select the mode this way (design §1.2):

1. **Explicit override wins.** If the user said `--bootstrap` / "make it
   publishable", or `--cut` / "ship 1.2.0" / "publish", honor it and skip
   detection.
2. **Else audit-first.** Run the readiness audit (read-only) and read the gap
   report's core status:

   ```bash
   ossctl audit --json || exit
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

**Non-recursion.** `/oss-release` invokes members; **no member invokes
`/oss-release`**. "Release intent" authorizes *entering* cut-release mode — it
never authorizes publishing. Publishing is gated separately at the approval
boundary below.

**Override selects the mode; it never disarms a gate.** `--cut` on a repo with an
incomplete core still fails the core gate below (with a reason) — an explicit
mode choice picks *which* path runs, not *whether* its preconditions hold.

## Bootstrap mode — sequence the members

Detect facts, score the gaps, checkpoint with the user, then sequence the
generators. The order matters — **CI before README** kills the badge cycle
(`/oss-ci` returns the workflow name + badge URL that `/oss-readme` consumes, so
the README is written exactly once):

```bash
ossctl facts --json || exit    # ecosystems, packages, CI, tags
ossctl audit --json || exit    # the gap report that drives sequencing
```

1. Present the gaps — **core gaps (must-fix) vs. recommended (offered)** — and
   get the user's go-ahead **before any file is written**.
2. Close the core: **`/oss-ci`** → **`/oss-readme`** (README + LICENSE).
3. Then the recommended set, scaled to the contract's `maturity` tier, serially
   (one reviewable diff at a time): **`/oss-changelog`** →
   **`/oss-contributing`** → **`/oss-security-policy`** →
   **`/oss-architecture`** (only when the contract opts in — its `docs_site` is
   not `none`, or the user explicitly asked for architecture docs).
4. Re-audit and report what landed and what remains:

   ```bash
   ossctl audit --json || exit
   ```

**Stop on member failure.** If a member fails, or the user rejects its diff,
**halt the sequence** — do not invoke a dependent member (notably: never run
`/oss-readme` if `/oss-ci` did not return its workflow name + badge URL, or it
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
This is `/oss-*` family canon (see `AGENTS.md` → "Cross-platform is a hard
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
- **Mirror `ossctl`'s own `dist-workspace.toml`** (repo root) as the reference
  shape: pinned `cargo-dist-version`, `ci = "github"`, `hosting = "github"`,
  `github-attestations = true`, `pr-run-mode = "skip"` (tag-triggered only).
- **`dist generate` is the sole author of `release.yml`** — edit config in
  `dist-workspace.toml`, then regenerate; never hand-edit the workflow. Owning
  `release.yml` (the tag-triggered publish/build/sign workflow) is the
  release-cut's job, **not** `/oss-ci`'s (which owns `ci*.yml`).

> **Status (engine gap).** The `ossctl` release engine does **not yet** generate
> `dist-workspace.toml` / `release.yml` from `distribution` — it currently
> consumes only `distribution.homebrew_tap` for the Homebrew adapter (see the
> `gh-release-ci-workflow` issue). Until that lands, the mapping above is the
> **documented default the generator (current or future) reads from the
> contract**, and any hand-driven release-infra setup MUST follow it. This makes
> the cross-platform (Mac + Linux) target set the binding default regardless of
> who authors the config.

**1. Gate.** First, check for an already-active run so two cuts never race — if
one is in flight, reconcile it (`resume` / `verify` below) instead of sealing a
second plan. Then require an approved contract, a complete core, and a clean
working tree:

```bash
ossctl release list --json || exit    # an in-flight run? resume/verify it, don't start a second
ossctl contract show --json --require-approved || exit
ossctl audit --json || exit           # refuse cut-release while the core is incomplete
```

`release plan` / `cut` are content-addressed and **refuse on repo drift**; a
dirty tree also risks sweeping uncommitted work into the release. Have the user
commit or stash first — do not stash or discard changes on their behalf.

**2. Decide the version (this skill's judgment — design §3.4).** `release plan`
takes the version as input and never derives it, so choosing it is your job.
Read the contract's `conventional_commits` and `versioning`. Find the last
release tag from `ossctl facts --json`, then read the commits since it:

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
  matching the contract's `versioning` scheme (not a bare `minor`/`patch`); if
  `release plan` rejects it, relay the error and re-prompt.

**3. Seal the plan.** Pass the chosen version; the binary computes a
content-addressed plan and **exits at the approval boundary** rather than
prompting (ADR-0001 §3):

```bash
ossctl release plan --version <VERSION> --json || exit
```

**4. Render the approval boundary (the one human checkpoint).** Show the user the
sealed `plan_id`, the version, the one shared tag, the changelog diff, and every
publish destination — all from the `release plan --json` payload. Publishing is
**irreversible** (crates.io/PyPI versions are permanent) — state that plainly and
require an explicit confirmation of this exact plan + version before proceeding.
This is the *only* approval prompt; step 2 merely settled the number.

**5. Cut, only after human approval.** Re-invoke with the sealed plan and the
same version; the cut refuses if the repo drifted from what the plan hashed, or
if the version does not match what was sealed. **Capture the `run_id` from its
output** — every reconciliation command below needs it:

```bash
ossctl release cut --plan <PLAN_ID> --version <VERSION> --json || exit
```

On a **drift refusal** the cut never started: have the user reconcile the working
tree, then re-run `release plan` (a new `plan_id`) and re-render the boundary.

**6. Interruptions and reconciliation.** A dropped network / OTP timeout /
one-of-N registry failure is recoverable from the journal — never re-publish by
hand. If the session died before you captured the `run_id`, recover it with
`ossctl release list --json`:

```bash
ossctl release list --json               # find the in-flight run_id
ossctl release show <RUN_ID> --json      # progress (live) or post-mortem
ossctl release resume <RUN_ID> --json    # reconcile + continue from the journal
ossctl release verify <RUN_ID> --json    # read-only reconcile vs. registry state
```

Report full success **or the precise partial state** — never present a
half-published release as wholly done or wholly failed. There is no automatic
rollback of an irreversible step; surface the concrete choices to the human
(resume the pending target, verify against the registry, or accept the skew and
complete the missing publish out of band).

## Success criteria

- **Bootstrap:** `ossctl audit --json` reports no blocking core gaps.
- **Cut-release:** the run reaches a terminal `published` + `tagged` state,
  confirmed by `ossctl release show <RUN_ID> --json`.
- The contract validates throughout: `ossctl contract validate --json` exits 0.
