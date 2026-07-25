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

## Status

**Private, early.** The founding architecture is decided and recorded in
[`docs/adr/`](docs/adr/); the workspace is being built out. Not yet published.

## License

MIT (intended). LICENSE to be added as the project firms up.
