---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: in-progress
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

## Comments

### 2026-08-17T04:04:54Z · @claude

COMPARATIVE EVIDENCE (2026-08-17): a sibling tool's tap formula, produced by cargo-dist rather than by ossctl's adapter, has the CORRECT shape — per-platform 'url' entries pointing at the published release binaries with their sha256, no toolchain dependency, and a real 'desc'. ossctl's engine-written formula is the source-build shape, and it does not even install. So the adapter's template is not merely a different choice from cargo-dist's; it is strictly worse on both axes at once.

This sharpens the decision in the section above. Two directions:
(a) fix the adapter's template to emit the prebuilt-binary shape cargo-dist already emits correctly;
(b) delegate the Homebrew leg to cargo-dist entirely — the same delegation already used for the GitHub Release — and reduce the engine's role to VERIFYING that the tap carries the released version.

(b) is the smaller and more honest system: cargo-dist demonstrably produces a correct formula today, and the engine's own attempt has been shipping a broken one for six releases without noticing. Weigh it against the 0.2.3 decision that deliberately made the leg self-sufficient (dropping the brew bump-formula-pr dependency) — that decision was about removing a fragile external CLI dependency, not about owning formula rendering, so (b) does not obviously contradict it. This is an architectural call worth recording, not a detail to settle inside a bugfix.

Token status verified: HOMEBREW_TAP_TOKEN has existed on this repository since 2026-08-05, so credential availability is not a constraint on either direction.
