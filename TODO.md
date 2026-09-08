# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)


_**Handoff updated 2026-09-06 after the publicize and release-preflight stint.** Main is
clean and pushed; current main CI `34047534365` is green. The supported command remains
`shipshape`, the published crates are `shipshape-core` and `shipshape-cli`, and durable
`ossctl` compatibility identifiers remain intentionally unchanged._

_**Live release:** Shipshape v0.12.1 is installed locally at commit `510276c`; `shipshape
doctor --json` is green. GitHub Release `v0.12.1` is published with exactly 11 assets and
main CI for its release commit is green. The maintained prebuilt set remains macOS arm64
and Linux musl arm64/x86_64; Intel macOS and Windows remain unsupported._

_**What landed:** the family now includes the thin `/shipshape-publicize` workflow and
binary-backed checks derived from the project-canon and Glasspad publicize passes. The
bundled installer supports Claude, pi and Codex as first-class targets under the current
canon contract. Release preflight recognizes observed inline GitHub Actions `push.tags`
sequences with direct Cargo publishes while preserving warnings for absent, malformed,
branch-only and non-inspectable publish paths. The completed issues are closed._

_**Release evidence and recovery:** the v0.12.0 engine run
`01M1KPK88J1S7WW65MABF7EW5H` first stopped safely in build because the pinned cargo-dist
executable was absent, before any publish or tag. It resumed with a verified disposable
cargo-dist 0.32.0, then observed both crates, the 11-asset Release, Homebrew, and remote-main
advancement. The disposable install and its temporary profile hook were removed. A
version-coupled post-bump test was repaired and CI returned green. The run is completed,
not a recovery candidate. Shipshape v0.12.1 was subsequently released and converged._

_**Prepared next stint:** implement `cargo-dist-preflight`. The product decision is to
validate every required executable and the `dist-workspace.toml`-pinned cargo-dist version
at `release cut` startup, after loading and validating the sealed plan but before creating
the release journal or applying the bump. Fail without mutation and provide an actionable
required-version diagnostic; do not auto-install cargo-dist. Re-check the dependency on
resume before entering a phase that needs it. `cargo-dist-dry-run-gap` is closed as the
directed duplicate. Use the live issuectl DAG for all execution mechanics._

_**Longer direction:** the next substantive milestone remains the 1.0 evidence gate: real
cuts for still-unproven fleet release shapes, a soak without new HIGH findings, then a
written compatibility/stability contract. Tests are not substitutes for observed cuts.
There is no parent epic for that gate; create one only if the maintainer requests a
checkable tracking artifact._

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
