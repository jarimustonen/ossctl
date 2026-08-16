---
created: 2026-08-04
updated: 2026-08-04
type: bug
status: fixed
priority: high
epic: ossctl-phase4-build
commits:
- hash: 2332c95
  summary: first-formula bootstrap (create vs bump) + tap/license threading
- hash: 11d5a5f
  summary: apply /llm-review + /assess-findings confirmed findings
closed: 2026-08-04
---

# homebrew-tap adapter has no first-formula bootstrap (bump-formula-pr can't create)

## Description

Found during ossctl's OWN 0.1.0 self-cut (stint #9). The homebrew adapter (crates/ossctl-core/src/release/adapters/homebrew.rs:88) always runs 'brew bump-formula-pr', which UPDATES an existing formula. On a fresh/empty tap with no <formula>.rb yet, there is nothing to bump, so the first release fails. Fundamentally the first formula also needs the published tarball's sha256, which only exists AFTER the GitHub release. 

The adapter needs a first-release path: detect the formula is absent in the tap and CREATE the initial .rb (url + sha256 from the release tarball, license, build/install stanza), committing/PRing it to the tap; only bump on subsequent releases. WORKAROUND USED: hand-created Formula/ossctl.rb in example-org/homebrew-ossctl (source-build formula) and verified 'brew install example-org/ossctl/ossctl' works. Future cuts can now bump-formula-pr it.

## Outcome (fixed)

`homebrew.rs` now branches on formula presence: a `homebrew-tap` target with a
configured tap probes `gh api .../contents/Formula/<name>.rb` through the injected
runner and, when the formula is **absent** (a genuine 404 only — auth/rate-limit/
network errors abort rather than mis-fire a create), takes the **create** path:
generate a source-build `.rb` (url + sha256 + license + cargo install stanza),
clone the tap into a private per-attempt scratch dir, write the file (create-new),
commit (with an explicit git identity), push a branch, and open a PR. When the
formula is **present** it keeps `brew bump-formula-pr`, now with `--url`/`--sha256`
threaded in. `homebrew-core` and tap-less targets keep the plain bump.

The tap + license are threaded additively: carried on `ReleasePlan` (copied from
the already content-addressed contract, so `plan_id` is unchanged), projected by
the coordinator into `ReleaseArtifacts.homebrew`. The coordinator now resolves the
slug/source-tarball/homebrew inputs up front and threads them into dry-run + build
(not only publish), so dry-run genuinely previews the chosen create-vs-bump path.
No `execute()` signature change; the receipt's pre-existing `remote_url` now
records the PR URL (no JSON-shape change).

Reviewed with `/llm-review` (gemini, gpt-5.6, opus-4-7, deepseek) + `/assess-findings`
(history/assessment-homebrew-first-formula.md); 6 confirmed findings applied.

**Known limitation (dogfood):** because the coordinator threads `sha256: None`
pre-tag (the GitHub tag archive does not exist until after publish), a first-formula
PR is opened as a **draft** carrying a `sha256` TODO — the maintainer fills the
digest once the tag is pushed, exactly like the 0.1.0 hand-fill. A real end-to-end
engine-driven tap bump therefore still needs the sha256 completed by hand for the
*first* formula; subsequent bumps are fully automated.

**Spin-offs filed:** `homebrew-adapter-fs-port` (fs write port),
`homebrew-create-resume-journaling` (sub-step journaling / remote reconcile for
safe resume), `homebrew-formula-non-rust` (drop the hardcoded cargo stanza).
