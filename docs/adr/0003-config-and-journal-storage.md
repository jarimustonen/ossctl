# 0003 — ossctl config artifact + release-cut journal/state storage

**Status:** Accepted
**Date:** 2026-07-25
**Authors:** Maintainer (decision owner); founding-architecture worktree (agent). Pressure-tested via `/llm-panel` (architect=gemini-3.1-pro, maintainability=gpt-5.6-sol, AI-first-CLI-ergonomics=deepseek-v4-pro, release-engineering=claude-opus-4-7).

> Companion to **ADR-0001** (founding spine) and **ADR-0002** (release engine). This ADR settles two storage questions: (1) that `OSS-RELEASE.md` stays the project-carried human-facing contract (and is *not* converted to `ossctl`'s own §8 tool config), and (2) the format, location, and resume/reconcile contract of the release-cut journal.

---

## Context

Two distinct kinds of "config/state" must not be conflated:

- **`OSS-RELEASE.md`** — the **project's** release contract: frontmatter = machine config, body = human rationale. Read by all 10 members only through the normalizer (`ossctl contract show`, ADR-0001). Its canonical-JSON shape is a schema-versioned inter-skill contract. Floors (design §3.1): no `release.model: auto` on a spike; a registry target requires a valid SPDX license; `slsa-l3` only at production; every health-badge needs its producer; `schema_version` bound; `changelog.fragment_dir` must stay inside the repo.
- **`ossctl`'s own tool config** (§8) — API URLs, profiles, credentials. At founding `ossctl` has **none** (it is repo-scoped and shells out to `cargo`/`npm`/`gh`, which carry their own auth).

The release cut (ADR-0002) is a partially-irreversible, resumable state machine whose progress must survive a network drop, an OTP timeout, or a one-of-N registry failure — and must be recoverable **without re-publishing what already landed**. `octl-core` proves the pattern: `manifest.json` + append-only events + an `applied_seq` watermark + an idempotent reducer, with append-then-apply atomicity (fsync event → apply to projection/manifest → advance watermark; a crash between fsync and apply replays as a clean no-op-or-apply).

The panel raised one correctness landmine repeatedly: a **literal `.git/` path is not a portable repo-state abstraction** (linked worktrees make `.git` a *file*; submodules, bare repos, and `GIT_DIR` overrides all break naive concatenation), and git-local state is **easy to lose** exactly when recovery is needed.

---

## Decision

### 1. `OSS-RELEASE.md` stays the project artifact

`OSS-RELEASE.md` remains the human-reviewed, project-carried contract at the repo root (frontmatter = config, body = rationale), authored by `/oss-init` and read via `ossctl contract show`. It is **not** migrated into `ossctl`'s §8 tool config — the two are separate concerns (project contract vs tool settings). `ossctl` exposes **no** `config` noun at founding (ADR-0001 reserves it for any future §8 settings, keeping `contract` for the project artifact). The normalizer materializes every default, expands `targets`, splits `versioning` into base + `versioning_pattern`, and enforces every floor **before** any external state changes; a config that crosses a floor never lands (design §3.3).

### 2. Journal format: event-sourced JSONL, mirroring `octl-core`

Per release run, `ossctl-core` writes:

- **`journal.jsonl`** — append-only, one self-contained JSON **event per line** (§2 JSONL), each carrying its own **`schema_version`**, a monotonic **`seq`**, a timestamp, and the event payload. Event classes (ADR-0002): `run_created`, `phase_entered`/`phase_completed { phase, outcome }`, `target_dry_run`, `target_built`, `target_published { PublishReceipt }`, `target_cancelled`, `tag_created_local`, `tag_pushed_remote`, `github_release_created`, `run_abandoned { reason }`.
- **`manifest.json`** — the materialized run state (run_id, contract snapshot hash / `plan_id`, target set, current phase, `applied_seq`, terminal status). The manifest is **disposable and reconstructable from `journal.jsonl`** — the journal is the single source of truth; if the manifest cannot be rebuilt from events, there are two sources of truth (forbidden).

**Atomicity (non-negotiable, from octl-core):** append-then-apply — fsync the journal event, THEN apply to the manifest, THEN advance `applied_seq`. The reducer is **deterministic and idempotent**: replaying the event stream against existing state is a clean no-op-or-apply, so a crash between append and apply is recovered by replay. Manifest writes use temp-file → flush → atomic rename → directory fsync. `target_published` receipts are written **per target before the next target is attempted** — never batch-written.

**Per-event schema versioning + forward tolerance:** each event carries its own `schema_version` (the journal is durable across `ossctl` upgrades — a run started under v0.3 must be resumable under v0.4). The reducer tolerates additive fields and **refuses an unknown *required* event schema with an actionable error rather than mutating state**.

### 3. Journal location: git-common-dir-local, single-active-cut lock

- **Location:** `$(git rev-parse --git-common-dir)/ossctl/releases/<run_id>/{journal.jsonl,manifest.json}` — resolved via git, **never** by concatenating `.git/`. Using the **common** git dir means all linked worktrees of one repo share one release-state root (a release is single-repo; a tag is shared), and submodules / bare repos / `GIT_DIR` overrides resolve correctly. Overridable with `--journal-dir` for CI or debugging. `run_id` is a **ULID**.
- **Single active cut per repo:** a file lock (`flock`) on `$(git rev-parse --git-common-dir)/ossctl/releases/.lock` taken at run creation. A concurrent `release cut`/`resume` fails fast, naming the active `run_id` — two processes must never execute or reconcile the same release concurrently.
- **`doctor` reports journal integrity:** for every entry `release list` knows about, `doctor` flags a **missing/orphaned journal** so a resume never silently looks "clean" against a wiped `.git`.

### 4. Resume/reconcile: remote state is ground truth; a documented state table

Resume does **not** trust the local journal as authoritative — the journal is an optimization; the **remote registry state is the ground truth** (so a run whose `.git`-local journal was wiped can still be reconciled from what the registries hold, via each adapter's `verify`, ADR-0002). Reconciliation is a **documented state table**, not prose:

| Journal says | `verify()` (ADR-0002) returns | Action |
|---|---|---|
| target published | `Matches` | done — skip (idempotent success) |
| target published | `Conflicts` (digest/version mismatch) | **hard stop** — someone/something else published; surface, do not overwrite |
| target published | `Missing` | ambiguous (deleted vs transient) — **hard stop + surface** for human decision; never blind re-publish |
| target published | `Unknown` (degenerate verify) | surface as unverifiable; require explicit human go-ahead to proceed |
| target *not* recorded | `Matches` | reconcile forward — a publish landed before its receipt fsynced; adopt it, continue |
| target *not* recorded | `Missing` | resume the publish for this target |
| target *not* recorded | `Unknown`, **publish phase reached** | surface as unverifiable; require explicit human go-ahead (a publish could have landed before its receipt fsynced) |
| target *not* recorded | `Unknown`, **publish phase never reached** | resume the publish for this target (the run failed in dry-run/build; nothing could have published) |
| tag `created_local` only | (git) tag not on remote | retry push (idempotent) |
| tag `pushed_remote`, no Release | GitHub Release absent | create the Release (idempotent) |

The two `not recorded × Unknown` rows are discriminated by a **publish-phase-reached** signal derived from the run state. `Unknown` is the tri-state degenerate outcome (an ecosystem the binary cannot query, e.g. rust/cargo, or a registry outage): before publish-all was ever entered, nothing could have landed on a registry, so requiring `--allow-unverified` there would be needlessly conservative — resume proceeds. Once publish-all was entered (a crash mid-publish, where a publish could have landed before its receipt fsynced), the row stays unverifiable pending the go-ahead. The signal never relaxes the `target published × Unknown` row: a receipt exists only because publish ran.

The signal **fails safe** — a wrong `false` is the dangerous direction (it would resume a publish blind), so it is over-inclusive. It is `true` on the phase records (`current_phase` is `Publish`-or-later, or any recorded phase barrier is `Publish`-or-later — a completed `Publish` barrier clears `current_phase` but leaves the durable phase record, so both are checked) **and** on any durable *effect* of the publish-or-later phases even when the phase records themselves were lost or never written (a crash between a registry side-effect and its fsync, a v1 journal, or a journal partially reconstructed under this remote-is-ground-truth contract): a landed receipt, a recorded CI-delegation, or a created tag. Any such effect is irrefutable proof publish ran, independent of whether the phase bookkeeping survived — which is what keeps the relaxation safe without depending on the coordinator's write-ahead ordering being audited at this layer.

Resume continues **from the first incomplete step**; "already-done and matching" is success, "exists and conflicting" is a hard stop, and there is **no auto-rollback** (ADR-0002). `release verify <run_id>` runs this reconciliation **read-only** (no mutation) so an operator can inspect "what actually landed" before choosing `resume` vs `abandon`.

---

## Consequences

**Positive**

- Release progress survives interruption and is recoverable **without double-publishing**: the per-target receipts + the reconciliation table make "did this land?" an answerable, deterministic question against the registries.
- State is **co-located with the repo it mutates** and correct under worktrees/submodules/bare repos (git-common-dir), while the lock prevents concurrent cuts from corrupting a run.
- Reusing the `octl-core` append-then-apply/idempotent-reducer pattern means the atomicity and replay semantics are borrowed from a proven implementation, not re-derived.
- `OSS-RELEASE.md` stays a human-reviewable artifact beside the code, and the tool-config namespace is left clean for a real §8 need.

**Costs / risks accepted**

- **Git-local state can still be lost** (a reclone, `rm -rf .git`, an ephemeral CI container). Accepted because remote-state reconciliation via `verify` is the authoritative recovery path, `doctor` flags orphaned runs, and `--journal-dir` allows relocating the journal to durable storage in CI. (A dedicated journal export/backup command is noted as a likely near-term follow-up, not a founding requirement.)
- **Two representations** (event log + materialized manifest). Accepted: the manifest is strictly derived and disposable, so there is one source of truth; the cost buys O(1) status queries for `release show`.
- **Reconciliation ambiguity** (`Missing` after a recorded publish) is resolved conservatively as a **hard stop + human surface**, never a blind re-publish — trading some automation for safety on irreversible steps.

## Amendment — 2026-08-17: durable sealed plan store

`release plan` also writes its content-addressed approval document to
`$(git rev-parse --git-common-dir)/ossctl/plans/<plan_id>.json`, adjacent to
`releases/`. Creation is immutable: an identical retry is a no-op and differing
content at an existing address is refused. Each document retains the canonical
seal pre-image and is re-hashed through the planner's sealing seam on load; a
mismatch is `plan_store_corrupt`. This deliberately makes `release plan` no
longer strictly side-effect-free, but the write is a git-common-dir cache only,
never a repository file. It lets `cut` and `resume` execute the approved plan
from its sealed checkout after later code fixes move the live tree.

## Amendment — 2026-08-17: publish-none is representable (`targets: []`)

The contract distinguishes an **omitted** `targets` key from an **explicit empty
sequence**. Omitted (or `targets: null`) expands to the ecosystem default, unchanged.
An explicit `targets: []` is the author's authoritative "never publish anywhere" and
survives normalization as an empty set — the machine-readable way to declare a
version-tracked but unpublished repo, without a phantom registry target in the
canonical JSON. An empty target set is a valid, honored state, not a
misconfiguration; the canonical shape is a JSON array either way, so no consumer
breaks and `schema_version` does not move.

Normalization additionally cross-reads the repo's `Cargo.toml` `publish` key as
evidence: a declared crates.io target for a crate that forbids publishing is a floor
error (the publish could never succeed), while a publish-none contract whose manifests
do not set `publish = false` is valid with a warning. Both are evidence-gated — no
readable manifest, no diagnostic — and every blind spot in the `publish` read errs
toward *publishable*, so the floor never fires on a guess. What a publish-none cut
does is ADR-0002's tag-only amendment.

**Rejected alternatives**

- **Home-dir / XDG state (`~/.ossctl/...`), as `orchestratectl` uses.** Rejected: `octl` keeps state in the home dir because *its* runs span multiple worktrees and repos; an `ossctl` release is intrinsically tied to **one** repo, so git-common-dir-local state co-locates the journal with the repo it mutates and avoids a home-dir hash-keyed lookup. (`--journal-dir` remains for the CI case.)
- **Literal `.git/ossctl/...` path.** Rejected: `.git` is a *file* in linked worktrees and is relocated by submodules / bare repos / `GIT_DIR`; the path must come from `git rev-parse --git-common-dir`.
- **A plain-text or single-JSON-document log.** Rejected: it cannot be appended atomically per event, cannot be `tail`/`jq`-streamed (§2), and cannot express the phase/receipt event model the resume contract needs.
- **A simpler "last completed step" checkpoint file (no event sourcing).** Rejected as insufficient for a partially-irreversible multi-target workflow: it cannot capture per-target publish receipts as facts, cannot distinguish reversible-attempt from point-of-no-return events, and cannot be replayed idempotently after a crash mid-write.
- **Trusting the local journal as authoritative on resume.** Rejected: the journal can be lost or lag the registries; remote `verify` must be the ground truth, with the journal as an optimization.
- **Converting `OSS-RELEASE.md` into `ossctl`'s §8 tool config.** Rejected: it is the *project's* human-facing contract, not the tool's settings; the two have different owners, lifecycles, and audiences (design §3, §8).
