# shipshape

Shipshape is the release and readiness engine behind the `/shipshape-*` Agent Skills:
an AI-first Rust CLI that takes a repository to open-source release quality and cuts
releases from it. The binary owns the contract normalizer and validator
(`OSS-RELEASE.md`), repo-fact detection, the readiness audit, and the resumable
per-ecosystem release-cut state machine. The prose skills are thin callers; when a skill
and the binary disagree, the binary is right and the skill is what gets fixed.

The product is live on its maintained channels: crates.io (`shipshape-core` and
`shipshape-cli`, the latter installing the `shipshape` binary), GitHub Releases through
cargo-dist (macOS arm64, Linux musl arm64 and x86_64, a `.sh` installer), and the
Homebrew tap. The earlier `ossctl` 0.10.x line is frozen history, not a fallback.
Version history is `CHANGELOG.md` plus git tags; `shipshape version --json` tells you
what is installed.

## The architecture is decided; read it before coding

The accepted ADRs under [`docs/adr/`](docs/adr/) are the spec, not background. Their
README summarizes what each one decides: the command taxonomy and core/CLI split
(0001), the adapter model, phase-barrier coordinator, sealed `plan_id` seam and the
mandatory post-cut verify barrier (0002), the contract, event-sourced journal and plan
store under `git-common-dir/ossctl/` (0003), one target = one publish unit (0004), and
the Shipshape migration with its compatibility boundary (0005). ADRs 0001–0004 keep the
historical `ossctl` name on purpose.

Two things in there are easy to mistake for leftovers. The `ossctl` path component, the
`ossctl.release-plan` seal domain, the `oss-changelog:*` markers and the
`OSS-RELEASE.md` filename are permanent compatibility identifiers (ADR-0005 §3). Renaming
any of them would strand every existing journal, sealed plan, changelog and downstream
contract across the fleet, so they stay. And `crates/shipshape-dist` is only a
non-published naming wrapper so cargo-dist emits `shipshape-*` archives; it is not a
third public API.

Open an `issuectl` issue before building a feature, and keep design within the ADRs.
Planning documents belong under the issue that needs them, so an issue is the place
where design can happen at all.

For any CLI surface work, the binding reference is the `/ai-first-cli-canon` skill from
`project-canon` (strict input validation, `--json`, JSONL logs, no prompts, informative
errors, composable commands). Update the canon at its source rather than keeping a
repo-local copy. The canonical JSON output shape is a schema-versioned compatibility
contract under that canon's §10: every family member reads it, so a breaking change is
a `schema_version` bump, never a silent edit.

## Operating policy

`/stint` reads this section for how work runs here.

**Green gate.** A unit counts as landed when these pass:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

`rust-toolchain.toml` pins the Rust release CI uses; running with an ambient `stable`
gives a different Clippy lint set and a green that CI will not reproduce. CI checks that
the `dtolnay/rust-toolchain` refs in `ci.yml` match the pin (the MSRV job is the one
exception), so bump both together. The doc build is the step people forget: a broken
intra-doc link fails CI's `docs` job even when tests pass. Tests that touch destination
verification control the observer rather than relying on host credentials or network:
an answering destination without the artifact is `Missing`, an unreachable or
unreadable one is `Unknown`, and `Unknown` is red.

**Git.** Rebase-then-push to `main`, including tag pushes, is granted here without
confirmation whenever the tree is clean and green (maintainer decision, 2026-08-05); this
overrides the global default that pushing is the user's step. What the grant does not
cover: force-pushing a shared branch, which rewrites history other machines and the
release engine have already observed, and pushing a red tree, which is how a bad release
gets cut.

**Releases are cut whenever there is something to release, autonomously, with no
go/no-go checkpoint** (maintainer decisions, 2026-08-05 and 2026-08-06). Do not stop to
ask; run the cut end to end and report each phase. The safety is structural, not
conversational: the sealed content-addressed plan, `dry-run-all` before any publish,
dependency-ordered and index-waited crates.io publishes, the undeclared-distribution
refusal, `resume` and `abandon`, and a verify phase whose green means every target was
observed at its destination. Trust the refusals. What is at stake: a crates.io publish
cannot be undone (yank is the only withdrawal and the number is burned), and a pushed
tag with a GitHub Release is effectively permanent for the installer and the formula
that pin it.

**What a cut needs that a local green does not show.** Both were learned when 0.10.0
was cut from a red tree (2026-08-21):

- CI on `main` is green (`gh run list --workflow=ci.yml --branch main --limit 3`). The
  local gate runs on one host; CI runs Linux and macOS.
- The `[Unreleased]` block of `CHANGELOG.md` is complete and is what you think it is.
  A parallel merge once landed a correct-looking entry inside an already-published
  version's block, which would have shipped wrong notes and rewritten a version already
  on crates.io. Union merge cannot detect this; only reading the block does.

The engine path is `shipshape release plan --bump <level>`, inspect the JSON, then
`shipshape release cut --plan <id>`; the phases run `bump → dry-run-all → build-all →
publish-all → tag → dist → verify → advance-branch`, where `advance-branch`
fast-forwards remote `main` to the bump commit and never force-pushes. The subcommand
help documents the flags and the resume and abandon semantics; a few facts sit outside
it. `plan` and `cut` refuse a binary not built from tree `HEAD`, so build a release
binary first (`cargo build --release -p shipshape-cli`). The cut also requires the exact
cargo-dist version pinned in `dist-workspace.toml` to be installed and never installs it
itself. `.github/workflows/release.yml` is generated by `scripts/release_workflow.py`
with a reviewed repo-local overlay; running `dist generate` against it would discard
that overlay. Should a phase fail past the point of resume, the manual fallback still
works: `gh workflow run publish-crates.yml` for the crates and a hand formula bump for
the tap. The one-time v0.11.0 recovery sequence in ADR-0005 and `docs/recovery/` is
history; bump runs are the normal path now.

**The macOS arm64 build runs on the maintainer's personal `hauis` self-hosted runner**
(the override at the end of `dist-workspace.toml`, a documented repo-local exception
that shipshape never generates for others). When that job fails with HTTP 400, the
cause is a stale git credential header on the runner:

```
ssh hauis 'git config --global --unset-all "http.https://github.com/.extraheader"'
gh run rerun <run-id> --failed
```

**Publish dispositions.** Every target declares who performs its publish, and the
engine's job follows from that. With `cargo-publish` and `homebrew-tap` the engine
publishes itself; the tap formula it writes carries the exact first-line marker
`# Generated by shipshape; do not edit by hand (template-version: 2)`, and the adapter
refuses to overwrite a formula without it. With `cargo-publish-ci` and `cargo-dist`, CI
publishes and the engine only observes, polling the destination with a bounded wait
(constants in `release/coordinator.rs`); delegated is not the same as unverified. An
authored `targets: []` is a tag-only cut that publishes nothing and passes verify
vacuously, which is categorically different from `Unknown`; a `distribution:` block
beside it is refused so cargo-dist cannot fire behind the plan's back.

Shipshape is the only live user of `homebrew-tap`, because its `dist-workspace.toml`
has no `publish-jobs`. Every other fleet repo (issuectl, glasspad, taskfleet,
project-canon) carries `publish-jobs = ["homebrew"]`, so cargo-dist writes their formula
on every tag and their contracts declare homebrew with `adapter: cargo-dist`. Declaring
`homebrew-tap` there creates two writers (this false-red'd an issuectl cut on a transient
503); omitting the target under-declares a channel users install from. The fleet
picture is in `homebase/issues/cross-repo-release-standardisation/audit-2026-08-17.md`.

**The sealed plan's hash format is versioned.** `release/plan.rs` documents what the
pre-image covers and the rule for evolving it: a change to the phase model or the
engine-owned bump edit set is a deliberate `SEAL_VERSION` bump, never a silent hash
change. After a bump, plans sealed by an older binary can no longer be cut and are
re-planned, though they still load so an interrupted run can `resume`.

**Cross-platform is a hard requirement.** Shipshape and every tool the family produces
offer `cargo install` plus prebuilt binaries for macOS arm64 and Linux musl arm64 and
x86_64. Windows and Intel macOS are deliberately unsupported as prebuilt channels
(reasons in `dist-workspace.toml` and the ADR-0005 amendment); a single-platform install
story is a release gap the audit reports.

**Two deliberate audit findings.** There is no Code of Conduct by maintainer decision;
`shipshape audit` listing it as a `recommended` gap is expected. Do not propose adding
one.

**Scope.** Shipshape is the generic, reusable engine. Cross-repository standardisation
and the maintainer's personal CI infrastructure are homebase concerns and stay out of
this repo's issues, `TODO.md` and handoffs; the `hauis` override above is the documented
exception.

**Hot files when work runs in parallel.** `Cargo.toml`, module `mod.rs` files, CLI
subcommand-dispatch files and the bundled-skill `CATALOG` in
`crates/shipshape-cli/src/skill.rs` are append-only rows that union-merge; brief workers
to keep every dep, declaration, arm and row, and expect to salvage the last adder's merge
by hand now and then. The shared-logic files are different: the canonical serde model in
`contract/schema.rs`, any existing `protocol/*.rs` module (a new file per unit is fine),
and the `release/coordinator.rs` plus `release/adapters/mod.rs` seam. Two workers
editing those produce semantic conflicts a merge tool cannot see, so sequence that work.
On the coordinator/adapters seam, start with the stronger worker model; a weaker one has
twice abandoned mid-unit there.

**What earns a place in the tracker** (maintainer decision, 2026-08-17): a finding whose
failure can actually occur in this project, on this path, with damage beyond an error
message. Provenance is a supporting signal, never the verdict; several models agreeing
correlates hardest on plausible-sounding generic advice. Cosmic-ray scenarios, duplicate
checks and hostile-input hardening where the only actor is the maintainer's own machine
are rejected. An unobserved finding is kept when the failure would be silent,
irreversible, reachable by a downstream user, or would contradict a documented
guarantee. Closing one records the reason and a reopen condition. The same standard
applies to a claimed blocker in a deferral: verify it rather than inherit it.

## Layout and conventions

The bundled skills live in `crates/shipshape-cli/skills/` and install with
`shipshape skill install`; its `--help` and `shipshape skill list --json` are the
complete contract, including runtimes, path overrides and the no-clobber rules.

`history/` is gitignored scratch for agents. Every directory follows the pattern
`AGENTS.md` for consolidated agent-relevant information, `CLAUDE.md` as a symlink to
it, and optional `AGENTS-<TOPIC>.md` splits.

Issues are managed by `issuectl` through the `/issue` skill: `issues/<slug>/item.md`
holds every issue and epic, status lives in frontmatter, `issues/AGENTS.md` carries the
schema and `.issuectl/AGENTS.md` the repo-local policy. Planning documents of any kind
go under their parent issue. Reporter and provenance are issuectl metadata to query
there, not lists to maintain in `TODO.md` or here; `TODO.md` carries only the current
handoff narrative and stable pointers, while issue state and scheduling stay in
issuectl.
