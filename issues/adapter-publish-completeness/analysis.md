# Adapter `publish()` completeness audit

_Read-only audit of the six ecosystem release adapters' `publish()` bodies, per
issue `adapter-publish-completeness`. Scopes the completion work; implements
none of it._

Scope audited: `crates/ossctl-core/src/release/adapters/{cargo,node,python,homebrew,go,binary}.rs`
plus the shared seam in `adapters/mod.rs` (`run_all`, `make_receipt`,
`EffectCtx`, `PublishReceipt`) and the coordinator's publish barrier
(`release/coordinator.rs`).

## How to read "REAL vs SKELETON" here

An adapter's `publish()` runs its command sequence through the injected
`CommandRunner` (`run_all`) and then returns a `PublishReceipt` via
`make_receipt`. The trait, dispatch, dry-run, `verify()`, phase-barriers and
journal-driven resume-skip are all **real and tested** (u-001 was correct on
that). The open question is only: *does the command sequence that `publish()`
actually runs perform the real upload, or is it a representative placeholder
that still returns a plausible receipt?*

- **REAL** — the command it runs is the genuine publish for that ecosystem
  (`cargo publish`, `twine upload`, `npm publish`, `goreleaser release`). A real
  cut with credentials present would actually publish.
- **SKELETON** — it runs a *representative* command that is missing required
  inputs (asset paths, tarball URL + sha256) and so cannot really publish, yet
  still returns a receipt. Marked `SKELETON:` in-code.
- **PARTIAL** — the struct backs several identities; some are REAL, at least one
  is SKELETON or honestly `Unsupported`.

Note two adapters (`cargo-dist`, `gh-action-pypi-publish`) return
`AdapterError::Unsupported` instead of a fake receipt — that is the *honest*
non-implementation the ADR wants, not a skeleton, and is treated as correct.

## Verdict table

| Adapter (file) | Identities | Verdict | Evidence |
|---|---|---|---|
| cargo.rs | `cargo-publish`, `cargo-dist` | **REAL** | Runs real `cargo publish -p <pkg>` `cargo.rs:109-112`; `cargo-dist` honestly `Unsupported` `cargo.rs:99-104` |
| python.rs | `twine`, `gh-action-pypi-publish` | **REAL** | Runs real `twine upload dist/*` `python.rs:91`; gh-action honestly `Unsupported` `python.rs:82-90` |
| go.rs | `goreleaser` | **REAL** (config-dependent) | Runs real `goreleaser release --clean` `go.rs:77-80` — does the whole publish, but only if a `.goreleaser.yaml` exists |
| node.rs | `npm-publish`, `changesets`, `release-please` | **PARTIAL** | `npm publish` / `changeset publish` real `node.rs:88-96`; `release-please github-release` is a representative skeleton that still returns an npm receipt `node.rs:91-94` |
| homebrew.rs | `homebrew-tap`, `homebrew-core` | **SKELETON** | `brew bump-formula-pr <pkg>` missing required `--url`/`--sha256` the coordinator must thread in `homebrew.rs:80-92`; receipt has no `remote_url` |
| binary.rs | `manual` (GitHub Releases) | **SKELETON** | `gh release upload <tag> --clobber` carries **no asset paths** — cannot upload anything as written `binary.rs:82-88` |

Summary: **3 REAL, 1 PARTIAL, 2 SKELETON.** The two pure skeletons (homebrew,
binary) share a root cause: their real command needs a concrete input (asset
paths / tarball URL + sha256) that the coordinator is expected to thread in but
does not yet.

## Cross-cutting gaps (apply to every adapter)

These are not per-ecosystem — they are structural and should be decided once,
before the per-adapter finishing work.

1. **No auth/token seam.** No adapter references a credential. All rely on
   *ambient* environment: `CARGO_REGISTRY_TOKEN` / `~/.cargo/credentials`,
   `~/.npmrc`, `TWINE_USERNAME`/`TWINE_PASSWORD` (or `__token__`),
   `GITHUB_TOKEN`. `EffectCtx` (`mod.rs:54-64`) has no secret provider. Decision
   needed: keep "ambient env is the contract" (document it, add a preflight
   check in `dry_run`) vs. thread a `SecretSource` port through `EffectCtx`.
   Recommendation: ambient env + a `doctor`/dry-run preflight is enough for v1;
   do **not** invent a secret port yet.

2. **`digest` is always `None`.** Every `publish()` calls
   `make_receipt(ctx, t, None, ...)` — the receipt digest field is never
   populated. Consequence: `classify_receipt` (`mod.rs:356-370`) can never reach
   `Conflicts`; a present remote version always resolves to `Matches`. This is a
   *known and documented* limitation of the `RegistryQuery` port (see the
   `remote_digest` note `mod.rs:338-343` and `reconcile.rs` header), not a bug to
   fix inside `publish()`. Completing cargo/twine could still parse the local
   checksum from CLI output to at least record a local digest.

3. **No idempotency self-guard against an already-published version.** The
   coordinator skips a target already in the journal's `published` set on a
   *resume* (`coordinator.rs:317-318`), and wave-3 `reconcile` does remote
   ground-truth. But on a **fresh run** whose version already exists remotely,
   each adapter behaves per its underlying CLI:
   - `cargo publish` / `npm publish` / `twine upload` → hard error → surfaces as
     `AdapterError::Command` (safe, but ugly: aborts the whole publish phase
     rather than treating "already there" as success).
   - `twine` should pass `--skip-existing`; `cargo`/`npm` have no such flag, so
     a pre-publish `verify`/`RegistryQuery` check is the clean guard.
   - `goreleaser release` and `gh release upload --clobber` may **silently
     overwrite/duplicate** rather than error — the more dangerous direction.
   Decision needed: is "coordinator pre-checks `verify()` before each publish and
   skips `Matches`" the intended idempotency layer? That belongs in the
   coordinator, not in six adapter bodies — worth confirming before finishing
   the adapters.

4. **Build artifact paths are deterministic skeleton names, not parsed output.**
   `build()` names artifacts by convention (e.g. `{pkg}-{ver}.crate`,
   `{pkg}-{ver}.tgz`) rather than reading them from tool output (marked
   `SKELETON:` in each `build`). The REAL publishers (cargo/twine/npm/goreleaser)
   don't need those paths — the CLI finds its own artifacts. But the two skeleton
   publishers (binary, homebrew) *do* need concrete paths/URLs, which is exactly
   what they're missing.

## Per-ecosystem sketches

### cargo (REAL)
- **State:** `cargo publish -p <pkg>` runs for real (no `--no-verify`, so a
  resume that enters publish still lets cargo verify the package). `cargo-dist`
  correctly returns `Unsupported` (its upload is the CI workflow).
- **What a real publish needs:** ambient `CARGO_REGISTRY_TOKEN` (or
  `~/.cargo/credentials`); crates.io upload = the `cargo publish` already run;
  idempotency = crates.io rejects a duplicate version (hard error today — could
  pre-check via `verify`); error handling already faithful via `run_all`.
- **What completing touches:** `cargo.rs` only — optionally parse the packaged
  `.crate` checksum from `cargo publish`/`cargo package` output into
  `receipt.digest`. External dep: `cargo`. Secret: `CARGO_REGISTRY_TOKEN`.
  Lowest-risk of the six; closest to done.

### python (REAL)
- **State:** `twine upload dist/*` runs for real; `gh-action-pypi-publish`
  correctly `Unsupported` (CI trusted-publisher). `twine` self-globs `dist/*`
  (arg is passed literally to the runner, not shell-expanded — fine, twine
  expands it).
- **What a real publish needs:** `TWINE_USERNAME=__token__` +
  `TWINE_PASSWORD=<pypi-token>` (ambient); PyPI upload = `twine upload`;
  idempotency = **add `--skip-existing`** (twine supports it natively — the one
  concrete code change worth making); error handling faithful.
- **What completing touches:** `python.rs` — add `--skip-existing` to the upload
  command; optionally record the sdist/wheel sha256. External dep: `twine`,
  `python -m build`. Secret: PyPI API token.

### go (REAL, config-dependent)
- **State:** `goreleaser release --clean` runs for real and does the entire
  publish (build + GitHub Release + artifact attach). No fake receipt.
- **What a real publish needs:** a committed `.goreleaser.yaml` in the repo (the
  hidden hard dependency — without it goreleaser errors); `GITHUB_TOKEN`
  (ambient); module availability is fronted by the immutable proxy so there is no
  registry upload; idempotency = goreleaser refuses to re-release an existing tag
  *unless* forced — but `--clean` **deletes `dist/`**, so a re-run rebuilds.
  `receipt.remote_url` is `None` (no natural single URL) — acceptable.
- **What completing touches:** mostly *documentation/preflight*, not `go.rs`
  code — assert `.goreleaser.yaml` presence in `dry_run` and surface a clear
  error if absent. External dep: `goreleaser`. Secret: `GITHUB_TOKEN`. Risk:
  behaviour is entirely delegated to a config file ossctl doesn't own.

### node (PARTIAL)
- **State:** `npm-publish` (`npm publish`) and `changesets` (`changeset
  publish`) are real. `release-please` runs `release-please github-release` as a
  **representative** command and still returns an npm receipt — the real publish
  is a CI job keyed off the release. This is the one true faithful-skeleton in an
  otherwise-real struct.
- **What a real publish needs:** `~/.npmrc` auth token (ambient); npm upload =
  `npm publish`; scoped public packages need `--access public` (**missing
  today** — a scoped first publish would fail); idempotency = npm rejects a
  duplicate version (hard error); custom registries make the hard-coded
  `remote_url` (`npmjs.com/...`, `node.rs:98-101`) wrong.
- **What completing touches:** `node.rs` — either make `release-please` honestly
  `Unsupported` (matching cargo-dist / gh-action, the cleaner story) or wire the
  genuine CI-driven step out of scope for a from-host cut; add `--access public`
  handling; derive `remote_url` from the target's registry instead of assuming
  npmjs. External dep: `npm` / `changeset` / `release-please`. Secret: npm token.
  Decision needed on the release-please identity.

### homebrew (SKELETON)
- **State:** `brew bump-formula-pr [--no-fork] <pkg>` runs, but the in-code
  `SKELETON:` note is explicit that a real bump needs the release **tarball URL +
  sha256** the coordinator will thread in — those are absent, so the command
  cannot actually open a correct formula PR in the general case. Receipt has no
  `remote_url`. `verify()` correctly returns `Unknown` (a tap isn't observable
  through `RegistryQuery`).
- **What a real publish needs:** `GITHUB_TOKEN` + a checkout/fork of the tap or
  `homebrew-core`; the mechanism is a **formula PR** carrying `--url <tarball>`
  and `--sha256 <digest>` (which requires the *upstream* artifact to already be
  published and hashed — an ordering dependency on the binary/GitHub-Release
  step); idempotency = a formula bump to an existing version is a no-op/duplicate
  PR; error handling faithful.
- **What completing touches:** `homebrew.rs` **and** the coordinator (must
  compute + thread the tarball URL and sha256 from the upstream release into the
  `AdapterTarget`/publish call — this input plumbing does not exist yet).
  External dep: `brew`, `git`, a tap repo. Secret: `GITHUB_TOKEN`. Riskiest:
  cross-target ordering + new coordinator plumbing + human-review PR semantics.

### binary (SKELETON)
- **State:** `gh release upload <tag> --clobber` runs but lists **no asset
  paths** — as written it uploads nothing (invalid/no-op). The `SKELETON:` note
  says the coordinator threads concrete asset paths in; that plumbing is absent.
  `build()` is intentionally empty (uploads artifacts produced elsewhere).
  `verify()` correctly returns `Unknown` (GitHub Releases aren't observable
  through `RegistryQuery`).
- **What a real publish needs:** `GITHUB_TOKEN` (ambient, via `gh`); mechanism =
  `gh release upload <tag> <asset>...`; the **asset paths** must be supplied
  (from whichever ecosystem build produced them — same threading problem as
  homebrew); idempotency = `--clobber` already makes re-upload safe (overwrites);
  error handling faithful.
- **What completing touches:** `binary.rs` **and** the coordinator (compute the
  asset path list — likely from prior adapters' `BuildArtifacts` — and thread it
  into publish). External dep: `gh`. Secret: `GITHUB_TOKEN`. Second-riskiest:
  needs the same new coordinator artifact-threading as homebrew.

## Recommendation on sequencing the completion work

The work is **not** six symmetric spinoffs. It splits cleanly into two shapes:

1. **Cross-cutting decisions first (one small design pass, no parallelism).**
   Settle, in the coordinator/ADR, before touching any adapter:
   - idempotency layer — confirm "coordinator pre-checks `verify()` and skips a
     `Matches` target" as the intended guard (keeps six adapter bodies simple);
   - auth policy — confirm "ambient env + dry-run preflight," no secret port;
   - the **artifact/URL threading** the two skeletons need (asset paths for
     binary, tarball-URL+sha256 for homebrew) — this is a coordinator +
     `AdapterTarget` change and is the single biggest missing piece. Both
     skeleton adapters block on it.
   These three are shared-logic (coordinator + `AdapterTarget`) and must be done
   serially, not in parallel worktrees.

2. **Then the per-adapter finishing, grouped by risk:**
   - **Near-done, safe, parallelisable (append-only per file):** cargo (record
     digest), python (add `--skip-existing`), go (add `.goreleaser.yaml`
     preflight). Each touches only its own file — a small fan-out is fine.
   - **One decision, then quick:** node — decide the `release-please` identity
     (recommend making it honestly `Unsupported`, matching cargo-dist), add
     `--access public`, fix `remote_url`. Do after the auth/idempotency decision.
   - **Riskiest, do last, sequentially, depends on the coordinator threading from
     step 1:** homebrew and binary. Both need the new asset/URL plumbing, both
     touch the coordinator, and homebrew additionally has cross-target ordering
     (its formula PR needs the binary release's tarball+sha256 to exist first).
     These are the two that genuinely need design decisions, not just code.

Suggested order: **coordinator threading + idempotency/auth decisions →
cargo/python/go (parallel) → node → binary → homebrew.** Do **not** spin off one
worktree per ecosystem up front; the two skeletons share a coordinator change
that must land before they can be finished, and finishing them in isolation would
duplicate that plumbing.

The issue stays **open** — this audit scopes the remaining work; none of it is
implemented here.
