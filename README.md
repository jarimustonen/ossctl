# ossctl

**Release & readiness coordinator** — an AI-first Rust CLI that takes any repository to
open-source release quality and cuts releases from it, safely and reproducibly.

`ossctl` is the deterministic engine behind a family of `/oss-*` Claude Code skills. It
owns the parts that must be exact and identical for every caller:

- **`ossctl contract show | validate`** — the single reader/validator of a project's
  `OSS-RELEASE.md` release contract (normalizes, materializes defaults, enforces floors).
- **`ossctl facts`** — deterministic repo-fact detection (ecosystems, packages, CI,
  tags, maturity).
- **`ossctl audit`** — readiness scoring against the gated core (README + LICENSE + CI)
  and the tier-scaled canon.
- **`ossctl release plan | cut | resume | verify | show | list | abandon`** — a
  resumable, journaled, per-ecosystem release-cut state machine with a sealed
  content-addressed approval plan.
- **`ossctl config path | show`** — inspect the project contract and release-journal
  locations ossctl resolves, with per-value provenance under `--json`.
- **`ossctl skill | doctor | version`** — companion-skill installer, self-diagnostics,
  and the version/schema surface.

The prose `/oss-*` skills (README/LICENSE authoring, CI, changelog, contributing,
security policy, architecture docs, distribution-channel generation, and the orchestrator)
ship bundled with the binary and are thin callers of it. The binary is the source of truth.
`/oss-dist` wraps `ossctl dist generate` to produce cargo-dist release infrastructure and
Homebrew tap/secret setup guidance from the approved contract.

## Install

`ossctl` runs on **macOS and Linux** (arm64 and x86_64). Pick whichever path fits:

- **Cargo** (source build, all platforms) — always current with the latest crates.io
  release:

  ```sh
  cargo install ossctl
  ```

The current release is published to [crates.io](https://crates.io/crates/ossctl), so
`cargo install` is the portable install path.

Prebuilt cross-platform binaries and a shell installer are produced with
[cargo-dist](https://opensource.axo.dev/cargo-dist/). Download the installer or a matching
archive from this repository's GitHub Releases for macOS (arm64 / x86_64) and Linux
(statically-linked `musl`, arm64 / x86_64).

## Status

**Public and shipping.** The founding architecture is recorded in
[`docs/adr/`](docs/adr/). Releases are available on
[crates.io](https://crates.io/crates/ossctl) (`ossctl` + `ossctl-core`) and this
repository's GitHub Releases. See [`CHANGELOG.md`](CHANGELOG.md) for the release history.

## Configuration inspection

`OSS-RELEASE.md` is a project-carried release contract, not a user-level ossctl
configuration file. ossctl currently has no `OSSCTL_*` configuration variables or
home-directory config. Inspect the actual paths and overrides it does use:

```sh
ossctl config path
ossctl config show --json
```

The report identifies the contract location (selected by `--repo-root` or the
current-directory default) and the release journal location (selected by
`--journal-dir` or derived from Git's common directory). If the current directory is
not a Git repository, the journal location is reported as unavailable rather than
inventing a fallback path.

## Companion skills

The `/oss-*` skills ship inside the binary and pin to its version (§17). Manage them
with `ossctl skill`:

```sh
ossctl skill list            # enumerate the bundled catalog
ossctl skill print oss-init  # stream one rendered SKILL.md to stdout
ossctl skill install         # install the whole catalog (add a NAME to scope to one)
```

`skill install` **dual-homes** each skill by default — the same `SKILL.md` is written
into **both** agent-runtime homes so it is discoverable whether you drive `ossctl`
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
