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
- **`ossctl skill | doctor | version`** — companion-skill installer, self-diagnostics,
  and the version/schema surface.

The prose `/oss-*` skills (README/LICENSE authoring, CI, changelog, contributing,
security policy, architecture docs, and the orchestrator) ship bundled with the binary
and are thin callers of it. The binary is the source of truth.

## Install

`ossctl` runs on **macOS and Linux** (arm64 and x86_64). Pick whichever path fits:

- **Cargo** (source build, all platforms) — always current with the latest crates.io
  release:

  ```sh
  cargo install ossctl
  ```

- **Homebrew** (macOS and Linuxbrew):

  ```sh
  brew install jarimustonen/ossctl/ossctl
  ```

The current release (**v0.1.0**) is published to [crates.io](https://crates.io/crates/ossctl)
and the `jarimustonen/ossctl` Homebrew tap — so `cargo install` and `brew install` are the
always-works paths today.

Prebuilt cross-platform binaries and a one-line shell installer are wired up via
[cargo-dist](https://opensource.axo.dev/cargo-dist/) and ship with the next tagged release
(v0.1.1). From that release on, the binary-download path will be:

```sh
curl -LsSf https://github.com/jarimustonen/ossctl/releases/latest/download/ossctl-installer.sh | sh
```

and prebuilt archives will be attached to each [GitHub Release](https://github.com/jarimustonen/ossctl/releases)
for macOS (arm64 / x86_64) and Linux (statically-linked `musl`, arm64 / x86_64).

## Status

**Public and shipping.** The founding architecture is recorded in
[`docs/adr/`](docs/adr/). **v0.1.0** is released — on
[crates.io](https://crates.io/crates/ossctl) (`ossctl` + `ossctl-core`), as GitHub
Release [`v0.1.0`](https://github.com/jarimustonen/ossctl/releases/tag/v0.1.0), and via
the `jarimustonen/ossctl` Homebrew tap. See [`CHANGELOG.md`](CHANGELOG.md) for the
release history.

## License

[MIT](LICENSE) — see the [`LICENSE`](LICENSE) file.
