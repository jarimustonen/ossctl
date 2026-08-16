# Contributing to ossctl

`ossctl` is an AI-first Rust CLI — the deterministic engine behind the `/oss-*` Claude
Code skill family. It takes a repository to open-source release quality and cuts releases
from it. Contributions are welcome.

> **Status: private, early.** The founding architecture is decided and recorded in
> [`docs/adr/`](docs/adr/). Read the ADRs before proposing a change to CLI surface or the
> release engine — they are the spec, not background.

## Reporting issues

This project tracks work with `issuectl`; issues
live in the repository under `issues/<slug>/item.md`. To report a bug or propose work, open
an issue there (via the `/issue` skill, or by adding an `issues/<slug>/item.md` in a PR) and
describe the expected vs. actual behaviour with steps to reproduce.

**Security vulnerabilities:** do **not** open a public issue. See [`SECURITY.md`](SECURITY.md)
for the private coordinated-disclosure process.

## Development setup

You need a stable Rust toolchain (the CI matrix pins an MSRV of 1.85 — see
[`.github/workflows/ci.yml`](.github/workflows/ci.yml)). Then:

```bash
cargo build --workspace
cargo run -q --bin ossctl -- version
```

The workspace has two crates: `ossctl-core` (the library) and `ossctl-cli` (the `ossctl`
binary). See [`AGENTS.md`](AGENTS.md) and the ADRs for the architecture.

## The green gate

Every change must pass this gate before it can merge — CI runs the same checks on every
pull request:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

Run all four locally before opening a PR.

## Commit messages

Commits follow a conventional style — `type(scope): summary` (`feat`, `fix`, `refactor`,
`docs`, `chore`) — and carry an `Issue: <slug>` trailer linking the issuectl issue the work
belongs to:

```
fix(facts): harden workspace member scanning

Issue: facts-workspace-members
```

Keep the summary imperative and concise. Automated commits (Dependabot, and commits emitted
by `ossctl release cut`) are exempt from the `Issue: <slug>` trailer requirement.

## Recording a changelog entry

The changelog is **curated** by the maintainer (`changelog.mode = curated` in
[`OSS-RELEASE.md`](OSS-RELEASE.md)) — no per-PR changelog action is required from
contributors. The `[Unreleased]` section of [`CHANGELOG.md`](CHANGELOG.md) is compiled at
release time; the maintainer curates entries from the closed issues your `Issue:` trailer
links.

## Pull requests

- Base your work on an up-to-date `main` and open the PR against `main`.
- Keep a PR focused on one issue; link the issue slug it closes.
- Ensure the green gate passes and that any new behaviour is covered by tests.
- The canonical `--json` output shape is a schema-versioned compatibility contract — a
  breaking change to it must bump `schema_version`, never change silently.

## Licensing

`ossctl` is licensed under the **MIT** license (see [`LICENSE`](LICENSE)). By contributing,
you agree that your contributions are licensed under the same terms (inbound = outbound).
