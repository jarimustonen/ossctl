# TODO

Pointers to open issues. Descriptions and plans live in the linked
`issues/<slug>/item.md` — do not duplicate them here. Full tracking via `issuectl`.

## 🔄 Continue here (handoff)


_**Handoff updated 2026-09-08 after the release-dependency preflight stint.** Main is
clean and pushed at the v0.12.2 bump commit `d1d48d6`; main CI `34219932215` is green.
The supported command remains `shipshape`, the published crates are `shipshape-core` and
`shipshape-cli`, and durable `ossctl` compatibility identifiers remain intentionally
unchanged._

_**Live release:** Shipshape v0.12.2 is installed locally at commit `d1d48d6`;
`shipshape doctor --json` is green. GitHub Release `v0.12.2` is published with exactly 11
assets. The engine observed both crates.io packages, the Release assets and Homebrew at
their destinations before advancing remote main. The maintained prebuilt set remains
macOS arm64 and Linux musl arm64/x86_64; Intel macOS and Windows remain unsupported._

_**What landed:** fresh release cuts now validate every executable required by the sealed
plan and require the exact cargo-dist version pinned by `dist-workspace.toml` before any
journal or bump mutation. Resume reconciles remote state first and re-checks only tools
needed by effective remaining work. Missing or mismatched cargo-dist produces an
actionable required-version diagnostic and is never auto-installed
(`cargo-dist-preflight`). The bundled skill catalog now enforces Pi's 1024 UTF-16-unit
description limit; `shipshape-changelog` renders at 853 units
(`shipshape-changelog-description-limit`). Both issues are fixed._

_**Release and convergence evidence:** engine run `01M20BC2AS09TY82KF8Y7755JB` completed
bump, dry-run-all, build-all, publish-all, tag, dist, verify and advance-branch for v0.12.2.
The cut used a checksum-verified disposable cargo-dist 0.32.0, which was removed afterward.
The fleet updater applied on Haapa; Shipshape v0.12.2 and the 853-unit installed skill were
also verified directly on Gertrud and Hauis. Brunhild was unreachable and remains
unverified until a later automatic fleet cycle._

_**Prepared next stint:** no accepted work is scheduled and no worker owns resumable work.
Start from the live issuectl DAG rather than manufacturing work from this narrative._

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
