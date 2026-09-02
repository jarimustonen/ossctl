# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)


_**Handoff updated 2026-09-02 after the Shipshape migration stint.** The ossctl →
Shipshape product migration is complete. Main is clean and pushed; main CI `32666524383`
is green. The supported command is `shipshape`, the crates are `shipshape-core` and
`shipshape-cli`, and the runtime skills are `/shipshape-*`._

_**Live release:** Shipshape v0.11.0 is observed on crates.io for both crates, on GitHub
Release `v0.11.0` with exactly 11 assets, and in `jarimustonen/homebrew-shipshape`.
Prebuilt support is deliberately limited to macOS arm64 and Linux musl arm64/x86_64;
Intel macOS and Windows are unsupported. The installed binary on the local Mac and Haapa
reports 0.11.0 / commit `63a55a5`, doctor is green, and all ten skills are in lockstep in
Claude, pi, and Codex. Legacy `/oss-*` entries and the frozen `ossctl 0.10.1` rollback
binary/tap have been retired on both managed hosts through Homebase._

_**Release recovery is closed, not resumable.** Replacement run
`01M0QQMW8Y6SWRR2G383M0KVJX` and the earlier collision run
`01M0QJKSEJZ0Z3JQGN0Q9ADE0Y` remain honestly `abandoned`; never resume either. The
three-platform fallback verified crates.io ×2, Release assets and Homebrew semantically.
Its retry path now validates the exact asset set, actual checksums, immutable-tag source
contents, installer, manifest topology and formula instead of demanding byte-identical
cargo-dist host/temp metadata._

_**Steady-state release configuration is restored.** `shipshape-core` is publishable and
the first crates.io target before `shipshape-cli`; future cuts must retain that dependency
order and index wait. The contract declares the three prebuilt platforms and the
engine-owned Shipshape tap. The repository coordinate, intake key and durable compatibility
identifiers intentionally remain `ossctl`; do not rename `OSS-RELEASE.md`, git-common-dir
state, journal/plan compatibility values or historical evidence._

_**Fleet ownership:** Homebase commits `99461227` and `837c8e93` make clean-host tap
convergence deterministic and retire only the provenance-matching rollback after Shipshape
passes version, doctor and full skill gates. A full Haapa fleet apply still reports unrelated
pre-existing `wilmai` installation and dotfile-link conflicts; Shipshape convergence itself
is independently verified. Do not force-replace those dotfiles from this repo._

_**Product direction after the rename:** the next substantive milestone remains the 1.0
evidence gate: real cuts for the still-unproven fleet release shapes, a soak without new
HIGH findings, then a written compatibility/stability contract. Tests are not substitutes
for observed cuts. There is currently no parent epic for that gate; create one only if the
maintainer wants a checkable tracking artifact._

_**Prepared next stint:** execute both accepted live-DAG issues. The release-preflight bug
must recognize the observed inline `tags: [...]` GitHub Actions trigger without weakening
the warning for a genuinely absent CI publish path. The publicize issue now has its required
second real data point from Glasspad; extract the thin `/shipshape-publicize` member and put
deterministic checks in the binary according to the issue's recorded A/B/C decision evidence.
Use the live DAG for execution mechanics rather than treating this prose as a schedule._

**Read first (the spec):** `docs/adr/000{1,2,3,4}-*.md` + the AGENTS.md operating policy
(engine recipe, hot files, issue standard).

## Scheduling

Canonical scheduling lives in `issuectl` frontmatter (`lane:`, `lane_seq:`, `blocked_by:`,
`collision:`). Do not maintain a markdown DAG or adjacent backlog in this file.

Use these views instead:

```bash
issuectl dag
issuectl dag --json
issuectl ls --status open
issuectl ls --status in-progress
```

`TODO.md` is only the session handoff and project notes; issue bodies and `issuectl dag`
are the source of truth.

## Backlog

[`ossctl-phase4-build`](issues/ossctl-phase4-build/item.md) — the founding extraction epic —
was closed as delivered on 2026-08-21. There is no parent epic now; open work stands on its
own issues. `issuectl list` for the live view.

## Piialiisan bugiraportit

- [x] 🐛 Piialiisan bugiraportti: Release bump plan accepts duplicate exact pins then cut fails — FIXED and released in 0.10.0 (SEAL_VERSION 6->7) — jari via Telegram ([`intake-bug-ossctl-d38ddf598fd5`](issues/intake-bug-ossctl-d38ddf598fd5/item.md))
- [x] 🐛 Piialiisan bugiraportti: Cargo-dist verifier reports existing GitHub Releases missing — closed as a duplicate; evidence folded into `verify-gh-release-missing` — jari via Telegram ([`intake-bug-ossctl-09cd3c1d03d0`](issues/intake-bug-ossctl-09cd3c1d03d0/item.md))
- [ ] 🐛 Piialiisan bugiraportti: homebrew/binary target verification fails: missing during cut, unknown … — jari via Telegram ([`intake-bug-ossctl-51f9c1ce4cfd`](issues/intake-bug-ossctl-51f9c1ce4cfd/item.md))
- [ ] 🐛 Piialiisan bugiraportti: Release plan misses tag-triggered Cargo publish workflow — jari via Telegram ([`intake-bug-ossctl-a5febb642dd7`](issues/intake-bug-ossctl-a5febb642dd7/item.md))
