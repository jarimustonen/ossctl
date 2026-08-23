# Shipshape

**Release & readiness coordinator** — an AI-first Rust CLI that takes any repository to
open-source release quality and cuts releases from it, safely and reproducibly.

`shipshape` is the deterministic engine behind a family of `/shipshape-*` Claude Code skills. It
owns the parts that must be exact and identical for every caller:

- **`shipshape contract show | validate`** — the single reader/validator of a project's
  `OSS-RELEASE.md` release contract (normalizes, materializes defaults, enforces floors).
- **`shipshape facts`** — deterministic repo-fact detection (ecosystems, packages, Cargo
  publish policy, CI, tags, maturity).
- **`shipshape audit`** — readiness scoring against the gated core (README + LICENSE + CI)
  and the tier-scaled canon.
- **`shipshape release plan | cut | resume | verify | show | list | abandon`** — a
  resumable, journaled, per-ecosystem release-cut state machine with a sealed
  content-addressed approval plan.
- **`shipshape config path | show`** — inspect the project contract and release-journal
  locations shipshape resolves, with per-value provenance under `--json`.
- **`shipshape skill | doctor | version`** — companion-skill installer, self-diagnostics,
  and the version/schema surface.

The prose `/shipshape-*` skills (README/LICENSE authoring, CI, changelog, contributing,
security policy, architecture docs, distribution-channel generation, and the orchestrator)
ship bundled with the binary and are thin callers of it. The binary is the source of truth.
`/shipshape-dist` wraps `shipshape dist generate` to produce cargo-dist release infrastructure and
Homebrew tap/secret setup guidance from the approved contract.

## Install

`shipshape` runs on **macOS and Linux** (arm64 and x86_64). Pick whichever path fits:

- **Cargo** (source build, all platforms) — use after the first verified Shipshape
  release reaches crates.io:

  ```sh
  cargo install shipshape
  ```

The manifests preserve the portable source-install path. The first crates.io release
will be published by the verified post-merge rollout in ADR-0005; until it completes,
retain the frozen `ossctl` 0.10.x installation rather than assuming the new channel is
live.

From the first Shipshape release onward, prebuilt cross-platform binaries and a shell
installer will be produced with [cargo-dist](https://opensource.axo.dev/cargo-dist/).
Download the installer or a matching archive from this repository's GitHub Releases for
macOS (arm64 / x86_64) and Linux (statically-linked `musl`, arm64 / x86_64).

## Status

**Source migration complete; first Shipshape release pending.** The founding
architecture is recorded in [`docs/adr/`](docs/adr/). After ADR-0005's rollout,
[crates.io](https://crates.io/crates/shipshape) will carry `shipshape` +
`shipshape-core`, with prebuilt artifacts on this repository's GitHub Releases. See
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
shipshape`. After verifying the first Shipshape release and `shipshape version --json`,
replace the declared fleet unit rather than leaving two persistent binaries.

The bundled skill catalog contains only the ten canonical `shipshape-*` names. A known
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

`skill install` **dual-homes** each skill by default — the same `SKILL.md` is written
into **both** agent-runtime homes so it is discoverable whether you drive `shipshape`
from Claude Code or from pi.dev:

- `~/.claude/skills/<name>/SKILL.md` — Claude Code
- `~/.pi/agent/skills/<name>/SKILL.md` — pi.dev (discovered as `/skill:<name>`)

Narrow the target with `--agent`:

| `--agent` | Writes to |
| --- | --- |
| *(omitted)* | Claude Code **and** pi.dev (dual-home — the default) |
| `claude` | `~/.claude/skills` only |
| `pi` | `~/.pi/agent/skills` only |
| `codex` | `~/.codex/prompts/<name>.md` (flat prompt file) |
| `all` | every known runtime (Claude + pi.dev + Codex) |

Installs are idempotent and version-guarded: a re-install of the same version
re-writes byte-identical content and emits no drift warning, and an on-disk copy
newer than the running binary is refused unless you pass `--force` (§17). `--dest
<PATH>` overrides the root directory (the per-runtime file shape still applies); when
several selected runtimes share a shape *and* that root — Claude and pi.dev both write
`<name>/SKILL.md` — they resolve to the same file, so the *write* collapses to one
file while the report still lists each requested runtime. The `--json` envelope
reports one `installed[]` row per requested target
(`{name, agent, dest_path, cli_version, schema_version}`) — the same field shape as
before; dual-home is additive to that shape, though note the **default now writes two
targets** (Claude + pi.dev) where it previously wrote one.

## License

[MIT](LICENSE) — see the [`LICENSE`](LICENSE) file.
