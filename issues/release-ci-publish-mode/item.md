---
created: 2026-08-10
updated: 2026-08-10
type: feature
status: open
priority: normal
---

# release engine: support a 'publish-in-CI / tag-only cut' release mode

## Description

**Friction hit while cutting glasspad 0.4.0 (2026-08-10) via `/oss-release`.**

`ossctl release plan --version 0.4.0` sealed a plan whose phases are:

```
dry-run-all → build-all → publish-all → tag → dist
```

`publish-all` performs a **local `cargo publish`** to crates.io. But glasspad's
release model (documented in its own AGENTS.md operating policy) is
**CI-triggered publish**: pushing the version tag fires two GitHub workflows —
`publish-crates.yml` (does the `cargo publish` in CI, using a repo-secret token)
and `release.yml` (cargo-dist builds binaries, creates the GitHub Release, pushes
the Homebrew formula). The repo deliberately forbids a local `cargo publish` (the
local `~/.cargo/credentials.toml` may be stale → 403; the CI token is the source
of truth).

There is no contract field to express this, so `ossctl release cut` cannot be
used for such a repo: its `publish-all` phase would either fail (no/stale local
token) or double-publish against the CI workflow. I had to **abandon the engine**
and fall back to the manual `git tag && git push` recipe.

### Observed vs. expected

- **Observed:** `release plan` always includes a local `publish-all` phase for a
  crates.io target with adapter `cargo-publish`; no way to say "the cut stops at
  the tag; CI does build + publish."
- **Expected:** a release mode where `release cut` bumps/finalizes/commits/**tags
  and pushes the tag**, then stops — leaving the actual build + publish to
  CI-on-tag-push. The engine would verify the published result (it already has
  `release verify` against the registry) rather than performing the publish
  locally.

### Sketch

- A contract field on the release block or per-target adapter — e.g.
  `release.publish = "ci-on-tag"` (vs. the current implied `"local"`), or a
  `cargo-publish-ci` adapter — that maps `publish-all` to a no-op-plus-verify and
  makes `tag` (push) the terminal actionable phase.
- Then `release verify <run>` reconciles crates.io / GitHub Release / tap state
  after CI finishes, closing the loop the engine currently closes by publishing
  itself.

### Notes

- This is the same tag-push→CI model used by cargo-dist's own `release.yml`; it's
  a common OSS pattern (publish from CI, not a dev laptop), so it's likely worth
  first-classing rather than treating as an exception.
- Minor adjacent nit: `ossctl release abandon <plan_id>` returns `run_not_found`
  for a plan that was sealed by `release plan` but never `cut` — a sealed plan
  isn't yet a "run". Not blocking; just mildly confusing when cleaning up an
  unused plan.
- Encountered with `ossctl 0.2.2`.

