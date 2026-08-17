---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: high
lane: release-safety
lane_seq: 0
---

# Generated Homebrew formula cannot install: cargo install on a virtual workspace manifest

## Description

## The defect

The Homebrew formula the release engine writes into the tap **cannot install**. Reproduced
2026-08-17:

```
$ brew upgrade ossctl
error: found a virtual manifest at `/private/tmp/.../ossctl-0.5.0/Cargo.toml`
       instead of a package manifest
```

The generated formula's install step is:

```ruby
depends_on "rust" => :build

def install
  system "cargo", "install", *std_cargo_args
end
```

`std_cargo_args` points cargo at the extracted tarball root. This repository is a **virtual
workspace** — its root `Cargo.toml` has no `[package]` — so `cargo install` refuses. The
template needs to name the binary package (`-p ossctl`, or a path argument pointing at the bin
crate). The generator emits this shape for any project it publishes, so **every multi-crate
workspace using ossctl's Homebrew leg gets an uninstallable formula**, not just this one.

## How long this has been shipping

The engine took over the tap write in 0.2.3 (2026-08-07, `homebrew-dist-brew-audit-fails`,
replacing `brew bump-formula-pr` with a direct tap write). Every release since — 0.2.3, 0.2.4,
0.2.5, 0.3.0, 0.4.0, 0.5.0 — reported its Homebrew leg green. The maintainer's own installed
binary is still **0.2.2**, the last version installed before the engine took over: upgrades have
been failing silently ever since, and the stale binary was mistaken for a stale-install habit
rather than a broken formula.

This is the failure mode the whole engine is built to prevent: the cut reports success, the
receipt is journaled, and the artifact does not work. The tap write succeeded — the formula's
*bytes* landed correctly — so nothing downstream of the write could notice. Nothing ever
installs what was published.

## Second, independent problem: source build vs. prebuilt binaries

Even once the manifest error is fixed, the formula builds **from source** and requires a Rust
toolchain (`depends_on "rust" => :build`). That contradicts this family's stated delivery policy
— prebuilt cross-platform binaries are published for exactly these platforms via cargo-dist and
attached to the GitHub Release, and Homebrew is documented as the **primary** install channel.
A source-building formula makes the primary channel the slowest one and imposes a toolchain
requirement on users who were promised a binary.

Decide deliberately which the tap should serve. Pointing the formula at the published release
binaries (`on_macos`/`on_arm` blocks over the cargo-dist assets) is the shape that matches the
policy; a source build is a fallback, not the default.

## Also worth fixing while in here

- `desc "ossctl"` is a placeholder — the formula description should come from the package
  description, since Homebrew surfaces it in search.
- The `test do` block runs `ossctl --version`, while this CLI's documented surface is the
  `version` subcommand. Verify the test actually exercises a real invocation.

## Acceptance

- `brew install` / `brew upgrade` of this tool succeeds from a clean state on macOS.
- The generated formula installs correctly for a **virtual-workspace, multi-crate** project —
  regression coverage on that shape, since it is the one that fails today.
- A deliberate, recorded decision on source build vs. prebuilt binaries, with the formula
  matching it.
- The release engine can tell that a published formula is installable, rather than only that the
  bytes were written. Whatever form that takes, "the tap write succeeded" must stop being
  accepted as proof the Homebrew leg worked.
