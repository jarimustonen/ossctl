# 0001 — ossctl founding architecture: CLI taxonomy, crate layout, binary↔skill boundary

**Status:** Accepted
**Date:** 2026-07-25
**Authors:** Jari Mustonen (decision owner); ossctl founding-architecture worktree (agent). Pressure-tested with a four-lens `/llm-panel` — architect (gemini-3.1-pro), maintainability (gpt-5.6-sol), AI-first-CLI-ergonomics (deepseek-v4-pro), release-engineering (claude-opus-4-7).

> **Provenance & staging note.** These are the **founding ADRs of the `ossctl` repo** (`~/Sources/ossctl`), written *before* the repo is scaffolded so the tool starts from a sound shape. They are authored here as staging and will be carried into `ossctl/docs/adr/` by `/create-project`. They realize — they do not re-open — the locked family design (`issues/oss-release-skill-family/design.md`) and conform to `AGENTS-AI-FIRST-CLI.md`. Companion decisions: **ADR-0002** (release engine + adapter model + sealed-plan approval seam) and **ADR-0003** (config artifact + journal/state storage). The three interlock; read them together.

---

## Context

`ossctl` extracts the **deterministic core** of the OSS-release skill family — 10 prose `/oss-*` Claude Code skills that take any repo to release quality — out of scattered Python-scripts-per-skill into **one AI-first Rust CLI** in its own repo. The prose skills stay skills but become **thin callers of the binary**. The binary is the **source of truth**; skills follow it (one-way sync, §17).

Two shipped sibling tools set unambiguous precedent, and we deliberately ground on them rather than invent:

- **`issuectl`** — cargo workspace, `issuectl-core` (lib) + `issuectl` (bin); a `skill` subcommand that embeds `SKILL.md` templates and pins `cli_version` via `{{TOKEN}}` substitution at install.
- **`orchestratectl`** — `octl-core` (lib) + `octl-cli` (bin); **bundles an entire `/worktree-*` skill family** (18 skills) under `crates/octl-cli/skills/<name>/SKILL.template.md`, version-pinned, installed via `orchestratectl skill install`; and an **event-sourced run journal** in `octl-core` (`manifest.json` + append-only events + an `applied_seq` watermark + an idempotent reducer, with append-then-apply atomicity).

The family already ships two deterministic Python scripts that migrate into `ossctl` and anchor the CLI taxonomy: `check-oss-release.py` (the config normalizer/validator — **the inter-skill contract**; every member reads `OSS-RELEASE.md` only through it and every reader's first act is to run it and abort on non-zero exit) and `infer-repo-facts.py` (the deterministic repo-fact detector). Their JSON output **shape** is a hard, schema-versioned compatibility contract that this taxonomy must preserve (the *subcommand names* may change; the *JSON shape* may not).

This ADR settles the **spine**: the command taxonomy, the workspace/crate layout, and which of the 10 members live in the binary vs stay skills. The release engine (adapters, the resumable state machine, the approval seam) and the storage model are settled in ADR-0002 and ADR-0003; this ADR references them where the spine touches them.

---

## Decision

### 1. CLI command taxonomy (noun-verb; §6–§7 strict)

```
ossctl contract show                 # THE normalizer / single reader of OSS-RELEASE.md:
                                     #   validates every field+enum+cross-field floor, materializes
                                     #   ALL defaults, expands `targets` from `ecosystems`, emits
                                     #   canonical schema-versioned JSON. Read-only. Non-zero exit +
                                     #   error envelope on invalid. Accepts --repo-root, --require-approved.
ossctl contract validate             # check-only gate: runs the identical normalization pipeline,
                                     #   emits ONLY pass/fail + the §10 error envelope (no canonical body).
ossctl facts                         # deterministic repo-fact detector → schema-versioned JSON
ossctl audit                         # readiness scoring → gap-report JSON (read-only; no repo writes)

ossctl release plan                  # compute + SEAL a content-addressed plan artifact (see ADR-0002)
ossctl release cut --plan <plan_id>  # execute a sealed plan; §12 JSONL stream; refuses on repo drift
ossctl release resume <run_id>       # reconcile + continue an interrupted run (journal + remote verify)
ossctl release verify <run_id>       # read-only reconcile of a run against remote registry state
ossctl release show <run_id>         # §12 progress query (live) / post-mortem (schema-distinguished)
ossctl release list                  # runs (active + past)
ossctl release abandon <run_id> --reason <text>   # terminal 'un-resumable' state, journaled

ossctl skill list | install | print  # §15–17 companion-skill installer; binary is source of truth
ossctl doctor                        # §18 read-only self-check (+ --fix twin)
ossctl version                       # §10 {version, commit, schema_version, supported_schemas, skills[]}
```

**Verb-vocabulary conformance (§7).** CRUD verbs used strictly: `show`, `list`. Documented exceptions, each an operation with **no CRUD equivalent** (§7 permits domain verbs with a written reason, as git has `commit`/`push`/`rebase --continue`):

| Verb | Why it is not `show`/`create`/`update`/`delete` |
|---|---|
| `contract validate` | An *assertion* (pass/fail, no data body), sibling to `doctor`; not a synonym for `show` (which emits the canonical document). Shares the exact normalize pipeline; discards the doc at the handler layer. |
| `facts`, `audit` | Read-only **analysis/detector** operations — a pure function of `(repo, HEAD)`. There is exactly one result each; nothing to `list`, `create`, `update`, or `delete`. §6 explicitly forbids inventing a noun layer for a one-result operation; sibling to `doctor`. |
| `release plan` | Produces the **sealed, content-addressed approval artifact** (ADR-0002). Not `create` — it mutates no external state; it is the read-only pre-image the human approves. |
| `release cut` | The release-engineering domain verb ("cut a release", the design's own `/oss-release-cut`). It runs the multi-phase, partially-irreversible publish; not `create`/`update`. |
| `release resume`, `verify`, `abandon` | Distinct **safety states** of an interrupted/partially-irreversible run (continue / read-only reconcile / terminal seal). Each has no CRUD equivalent and each **must be first-class and guessable** — the release-engineering and ergonomics lenses both required them explicit rather than folded into flags (see Consequences → the release surface). |
| `skill`, `doctor`, `version` | §15–18 sanctioned. |

**`contract`, not `config` (the panel's near-unanimous override of the initial proposal).** `OSS-RELEASE.md` is the **project's release contract**, and the design already names it "the inter-skill contract" throughout. `config` is reserved by §8 for the *tool's own* persistent settings (API URLs, profiles, credentials). Naming the project artifact `config` would appropriate that namespace on the permanent bet that `ossctl` never gains tool settings — and if it ever does, the collision forces a breaking rename or a lasting deviation from every sibling tool. `ossctl contract show|validate` names the artifact boundary honestly; `ossctl config path|show` stays available and §8-conformant for any future tool settings. At founding `ossctl` has **no §8 persistent tool config**, so `config` is simply absent from the surface rather than overloaded.

**`contract show` *is* the normalizer** (the Python `normalize` verb is not preserved). In an AI-first CLI, *showing* a config always returns its **canonical** form (validated, defaulted, `targets` expanded) — that is exactly what the 10 skills need, and what §2/§10 demand of structured output. Keeping a separate `normalize` verb would create two public routes to one representation and double the skill-template / test / help-sync burden. `contract show` and `contract validate` are two presentation modes over **one** normalization function; validate exists so CI gates and the file-ownership lint can assert validity without the §13 cost of a full JSON dump.

### 2. Crate / workspace layout

A **cargo workspace**, mirroring both siblings:

```
ossctl/
├─ Cargo.toml                         # [workspace] members = core, cli; shared [workspace.package]/lints/dist
├─ crates/
│  ├─ ossctl-core/                    # library — all deterministic logic; NO clap, NO stdout formatting
│  │  └─ src/
│  │     ├─ lib.rs
│  │     ├─ contract/                 # OSS-RELEASE.md schema + normalizer (port of check-oss-release.py)
│  │     │  ├─ schema.rs              #   serde types = the ONE canonical model (see ADR-0003)
│  │     │  ├─ normalize.rs           #   validate + materialize defaults + expand targets
│  │     │  └─ spdx.rs                #   vendored SPDX grammar+id check
│  │     ├─ facts/                    # repo-fact detector (port of infer-repo-facts.py) behind repo ports
│  │     ├─ audit/                    # readiness scoring over normalized contract + facts (pure rules)
│  │     ├─ release/                  # see ADR-0002
│  │     │  ├─ plan.rs                #   content-addressed sealed plan
│  │     │  ├─ coordinator.rs         #   phase barriers, ordering, tag ownership
│  │     │  ├─ journal.rs             #   event-sourced journal (see ADR-0003)
│  │     │  ├─ reconcile.rs           #   resume/verify state table
│  │     │  └─ adapters/              #   one module per ecosystem behind the ReleaseAdapter trait
│  │     ├─ protocol/                 # versioned public envelopes + JSON/JSONL DTOs (§10/§12)
│  │     └─ ports.rs                  # injected effects: CommandRunner, Clock, IdGen, RegistryQuery, Fs/Git
│  └─ ossctl-cli/                     # binary — clap surface, handlers, rendering, doctor, skill installer
│     ├─ src/{main.rs, cli.rs, help.rs, output.rs, doctor/, skill.rs, <noun handlers>}
│     └─ skills/<name>/SKILL.template.md   # the 10 bundled /oss-* skills, version-pinned
└─ docs/adr/000{1,2,3}-*.md           # these ADRs
```

**Two crates at founding, not more.** `ossctl-core` (all logic, unit-testable without spawning the binary) + `ossctl-cli` (clap + I/O + skill installer). This matches `issuectl-core`/`issuectl` and `octl-core`/`octl-cli` exactly — the proven shape at comparable complexity. `ossctl-core` is organized into the **domain modules** above with all external effects behind **injected ports** (`CommandRunner`, `Clock`, `IdGen`, `RegistryQuery`, `Fs/Git`) so each domain (contract / facts / audit / release / adapters / journal) is testable in isolation without touching the real filesystem, git, network, or clock. A later split into domain **crates** (`ossctl-contract`, `ossctl-release`, `ossctl-adapters`, `ossctl-protocol`, …) is a **mechanical module→crate promotion** and is the sanctioned escape hatch **if and when** `ossctl-core`'s compile/test time or coupling actually hurt — we do not pre-fragment at founding (see Consequences → rejected: eager domain-crate split).

**One canonical model, split from the wire protocol.** The `contract/schema.rs` serde types are the single normalization model that `show`, `validate`, `audit`, `facts` consumers, and release planning all use — no second parser anywhere. Public JSON/JSONL DTOs live in `protocol/` and are versioned independently of internal domain types, so `ossctl-core` can refactor internals without a wire break (§10).

### 3. Binary ↔ skill boundary (the 10 members)

| Member | Home | What moves / stays |
|---|---|---|
| **normalizer** (was `check-oss-release.py`) | **binary** | `contract show` / `contract validate` |
| **repo-fact detector** (was `infer-repo-facts.py`) | **binary** | `ossctl facts` |
| `/oss-readiness` | **hybrid** — thin skill over binary | scoring engine → `ossctl audit`; skill wraps it for user-talk/sequencing |
| `/oss-release-cut` | **hybrid** — mechanics in binary, judgment in skill | `ossctl release plan/cut/resume/verify/…` own mechanics+journal+adapters; skill owns the conversational SemVer-bump decision (the `conventional_commits:false` case, design §3.4) and renders the approval instruction. **The binary never prompts (§3): it `plan`s and exits at the approval boundary; the caller re-invokes `release cut --plan <plan_id>` to execute** (ADR-0002 makes this seam safe via the content-addressed, drift-checked plan). |
| `/oss-init` | **hybrid** — authoring in skill, mechanics in binary | judgment-heavy draft authoring + `## Rationale` stay a skill; its two scripts become `ossctl facts` + `ossctl contract validate`; the skill shells out instead of bundling Python. |
| `/oss-readme` (README+LICENSE) | **skill** | prose/judgment; reads config via `ossctl contract show` |
| `/oss-ci` (workflow YAML) | **skill** | tier/ecosystem-tuned templated judgment; reads `contract show` |
| `/oss-changelog` | **skill** | wraps `issuectl changelog`; structural marker ops |
| `/oss-contributing`, `/oss-security-policy` | **skill** | templated emission + threat-signal detection judgment |
| `/oss-architecture` | **skill** | opt-in docs; leans on `/worktree-technical-decision` |
| `/oss-release` (orchestrator/router) | **skill** | mode detection, sequencing, user-talk; calls `ossctl audit`/`contract show`, sequences member skills, hands off to `release plan`/`cut` |

**The line:** deterministic, reproducible, verifiable machinery → **binary**; judgment, prose generation, sequencing, and user conversation → **skill**; anything that is *both* → **hybrid**, with the deterministic half in the binary and the judgment half in the skill.

**How skills invoke the binary.** Every reader's first act is `ossctl contract show --json` (or `--require-approved` for mutating members) and **abort on non-zero exit** — gate on the exit code, never re-derive a default from prose. Streaming release commands use `--output=jsonl` (§12). This replaces the `python3 check-oss-release.py normalize` call the family currently makes.

**All 10 skills relocate into `ossctl`.** They live at `crates/ossctl-cli/skills/<name>/SKILL.template.md`, carry `cli_version:` + `schema_version:` frontmatter (§17), and install via `ossctl skill install` — exactly how `orchestratectl` ships the `/worktree-*` family. The family is one release unit: binary + its operating manuals. This kills the manual two-repo synchronization that leaving the prose members in homebase would impose. Lockstep is **mechanically enforced in CI** (§17): parse each bundled skill's frontmatter, verify every referenced `ossctl` subcommand/flag exists against the `--help --json` snapshot, verify the `cli_version` token substituted, and golden-test `skill install` + `skill print`. `/oss-init`'s current home (`SKILL.md` + `SCHEMA.md` in homebase dotfiles) migrates; its Python scripts are deleted in favor of the binary subcommands.

---

## Consequences

**Positive**

- One binary owns the inter-skill contract: the normalizer, fact detection, audit scoring, and release mechanics are deterministic, unit-tested, and identical across every caller — the "two agents parse the same YAML differently" failure the design feared cannot occur.
- The taxonomy is guessable and §7-faithful: an agent that knows the family reads `contract show`, `facts`, `audit`, `release plan|cut|resume|show`, `skill`, `doctor`, `version` and hits the right verb each time. No `config` overload, no synonym for the normalizer.
- The two-crate workspace matches both siblings, so the crate scaffolding, `skill` subcommand, dist config, and CI are near-copyable rather than novel.
- Relocating all 10 skills into the binary makes the family a single, mechanically-verifiable release unit — no cross-repo drift.

**Costs / risks accepted**

- **Prose-only skill fixes now require a binary release** (the lockstep cost of bundling). Accepted as an intentional, testable cost; mitigated by supporting skill-only patch releases in the release pipeline.
- **`ossctl-core` carries many concerns.** Accepted at founding (matches octl-core) and bounded by strict internal module boundaries + injected ports; the domain-crate split remains a cheap later move if it hurts (a documented, non-breaking promotion).
- **Migration debt** from renaming the Python `normalize`→`contract show` and relocating skills. One-time, absorbed by the greenfield scaffold; the JSON *shape* is preserved so downstream logic is unchanged.

**Rejected alternatives**

- **`ossctl config show` for `OSS-RELEASE.md`** (the initial proposal). Rejected — near-unanimous panel finding: it appropriates the §8 tool-config namespace and creates a breaking collision the moment tool settings appear. `contract` names the artifact honestly and frees `config` for its §8 role.
- **Keep `normalize` as an explicit domain verb** alongside `show`. Rejected: two public routes to one canonical representation; doubles help/skill/test sync burden. `contract show` *is* the canonical read; `contract validate` covers the pass/fail gate.
- **A noun layer for facts/audit (`facts show` / `audit show`)** to future-proof against sub-operations (maintainability lens). Rejected for founding: each is a one-result pure function of `(repo, HEAD)` with nothing to `list`/`create`; §6 warns against a one-result noun layer. If audit results are ever *persisted* as history, that is a genuinely new resource (`audit-run`) that earns its own noun **without** breaking the `audit` verb — so the feared migration is avoidable. (Preserved minor disagreement; not a blocker.)
- **Single binary crate (no `-core` library).** Rejected: forbids unit-testing the normalizer/adapters/journal without spawning the process, and diverges from both siblings.
- **Eager domain-crate split** (`ossctl-contract` / `ossctl-repo` / `ossctl-audit` / `ossctl-release` / `ossctl-adapters` / `ossctl-protocol` at founding, per the maintainability lens). Rejected as premature: module boundaries + injected ports already deliver the isolation/testability; a crate split adds build-graph and versioning overhead before the coupling has proven painful. Kept explicitly as the sanctioned growth path. (Recorded as a genuine trade-off.)
- **Leave the pure-generative prose members (`/oss-readme`, `/oss-ci`, …) in homebase, bundle only the binary-coupled skills.** Rejected: they still reference command names, output fields, and version expectations, so splitting the family across repos re-introduces the manual two-repo sync that bundling exists to remove.
