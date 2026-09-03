# Shipshape

**Release & readiness coordinator** — an AI-first Rust CLI that takes any repository to
open-source release quality and cuts releases from it, safely and reproducibly.

`shipshape` is the deterministic engine behind a family of `/shipshape-*` Agent Skills. It
owns the parts that must be exact and identical for every caller:

- **`shipshape contract show | validate`** — the single reader/validator of a project's
  `OSS-RELEASE.md` release contract (normalizes, materializes defaults, enforces floors).
- **`shipshape facts`** — deterministic repo-fact detection (ecosystems, packages, Cargo
  publish policy, CI, tags, maturity).
- **`shipshape audit`** — readiness scoring against the gated core (README + LICENSE + CI),
  the tier-scaled canon, and deterministic public-front-door checks used by
  `/shipshape-publicize`.
- **`shipshape release plan | cut | resume | verify | show | list | abandon`** — a
  resumable, journaled, per-ecosystem release-cut state machine with a sealed
  content-addressed approval plan.
- **`shipshape config path | show`** — inspect the project contract and release-journal
  locations shipshape resolves, with per-value provenance under `--json`.
- **`shipshape skill | doctor | version`** — companion-skill installer, self-diagnostics,
  and the version/schema surface.

The prose `/shipshape-*` skills (README/LICENSE authoring, CI, changelog, contributing,
security policy, architecture docs, distribution-channel generation, publicizing, and the release orchestrator)
ship bundled with the binary and are thin callers of it. The binary is the source of truth.
`/shipshape-dist` wraps `shipshape dist generate` to produce cargo-dist release infrastructure and
Homebrew tap/secret setup guidance from the approved contract.

## Install

`shipshape` runs on **macOS arm64** and **Linux arm64/x86_64**. Pick whichever path fits:

- **Cargo** (source build, all platforms) — use after the first verified Shipshape
  release reaches crates.io:

  ```sh
  cargo install shipshape-cli
  ```

The manifests preserve the portable source-install path. The first crates.io release
will be published by the verified post-merge rollout in ADR-0005; until it completes,
retain the frozen `ossctl` 0.10.x installation rather than assuming the new channel is
live.

From the first Shipshape release onward, prebuilt cross-platform binaries and a shell
installer will be produced with [cargo-dist](https://opensource.axo.dev/cargo-dist/).
Download the installer or a matching archive from this repository's GitHub Releases for
macOS arm64 and Linux (statically-linked `musl`, arm64 / x86_64). Intel macOS and Windows
receive no prebuilt artifacts; source installation remains available wherever the Rust
workspace and its dependencies build.

## Status

**Source migration complete; first Shipshape release pending.** The founding
architecture is recorded in [`docs/adr/`](docs/adr/). After ADR-0005's rollout,
[crates.io](https://crates.io/crates/shipshape-cli) will carry `shipshape-cli` +
`shipshape-core`, with prebuilt artifacts on this repository's GitHub Releases. The
`shipshape-cli` package installs the command `shipshape`; product, executable, release
asset, and formula names remain Shipshape. See
[`CHANGELOG.md`](CHANGELOG.md) for the release history.

## Repository facts

`shipshape facts --json` reports the repository evidence used by the detector and
normalizer. Its additive `data.cargo_publish` array lists every discovered Cargo
package manifest as `{manifest, package, policy}`. `policy` is `allowed`, `forbidden`,
or `unknown`; workspace-level `publish` inheritance is resolved before emission.
The contract normalizer uses this same evidence for its crates.io publish floor, so
an operator can inspect exactly what caused a validation refusal:

```sh
shipshape facts --json | jq '.data.cargo_publish'
```

## Configuration inspection

`OSS-RELEASE.md` is a project-carried release contract, not a user-level shipshape
configuration file. Shipshape currently has no `SHIPSHAPE_*` persistent configuration
variables or home-directory config. Inspect the actual paths and overrides it does use:

```sh
shipshape config path
shipshape config show --json
```

The report identifies the contract location (selected by `--repo-root` or the
current-directory default) and the release journal location (selected by
`--journal-dir` or derived from Git's common directory). If the current directory is
not a Git repository, the journal location is reported as unavailable rather than
inventing a fallback path. Release state deliberately remains under
`<git-common-dir>/ossctl/{plans,releases}`: this legacy-compatible machine namespace lets
Shipshape resume sealed plans and journals created by ossctl without copying state or
splitting the single-cut lock.

## Migration from ossctl

`ossctl` and `ossctl-core` 0.10.x remain available but are frozen; they are not aliases
for the maintained Shipshape packages. Install the new command with `cargo install
shipshape-cli`; Cargo installs its declared `shipshape` binary. After verifying the first
Shipshape release and `shipshape version --json`,
replace the declared fleet unit rather than leaving two persistent binaries.

The bundled skill catalog contains only the eleven canonical `shipshape-*` names. A known
`oss-*` name gets an actionable `skill_renamed` refusal. Install and verify the complete
new catalog before explicitly removing old runtime files; Shipshape never deletes files
outside the requested installation destination.

After the verified rollout, Homebrew users move with `brew uninstall ossctl`, `brew
untap jarimustonen/ossctl`, then `brew install jarimustonen/shipshape/shipshape`. The new
shell installer writes `shipshape`; remove an installer-managed stale `ossctl` only after
verifying the replacement. The full channel and machine-convergence sequence is recorded
in [ADR-0005](docs/adr/0005-shipshape-product-migration.md).

## Companion skills

The `/shipshape-*` skills ship inside the binary and pin to its version (§17). Manage them
with `shipshape skill`:

```sh
shipshape skill list            # enumerate the bundled catalog
shipshape skill print shipshape-init  # stream one rendered SKILL.md to stdout
shipshape skill install         # install the whole catalog (add a NAME to scope to one)
```

`skill install` targets **all three maintained agents by default**, exactly like an
explicit `--agent all`:

- `~/.claude/skills/<name>/SKILL.md` — Claude Code native Agent Skill tree
- `~/.pi/agent/skills/<name>/SKILL.md` — pi.dev native Agent Skill tree
- `~/.codex/prompts/<name>.md` — self-contained Codex prompt

Pass `--agent claude|pi|codex` to narrow the install to one runtime. Canonical
`--target <DIR>` replaces the install base while preserving those native paths, which
makes a repository root or disposable test directory safe to target. The older `--dest
<DIR>` remains compatible: it names the already-resolved skills directory directly and
therefore omits the `.claude` / `.pi` / `.codex` prefix. The two overrides are mutually
exclusive.

Installs are non-interactive, idempotent, and no-clobber by default. A managed older
copy upgrades with a warning; an unmanaged, malformed, non-regular, or newer destination
is refused unless `--force` safely applies. `--dry-run` performs the same complete
preflight and emits a `would[]` planning envelope without creating directories or files.
Use `--json` for the schema-versioned result. `skill list --json` advertises the supported
agents, exact layouts, selector/default, target, dry-run, force, and no-clobber
capabilities so callers can inspect the contract without mutating disk.

## License

[MIT](LICENSE) — see the [`LICENSE`](LICENSE) file.
