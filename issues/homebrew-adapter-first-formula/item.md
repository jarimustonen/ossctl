---
created: 2026-08-04
updated: 2026-08-04
type: bug
status: open
priority: high
epic: ossctl-phase4-build
---

# homebrew-tap adapter has no first-formula bootstrap (bump-formula-pr can't create)

## Description

Found during ossctl's OWN 0.1.0 self-cut (stint #9). The homebrew adapter (crates/ossctl-core/src/release/adapters/homebrew.rs:88) always runs 'brew bump-formula-pr', which UPDATES an existing formula. On a fresh/empty tap with no <formula>.rb yet, there is nothing to bump, so the first release fails. Fundamentally the first formula also needs the published tarball's sha256, which only exists AFTER the GitHub release. 

The adapter needs a first-release path: detect the formula is absent in the tap and CREATE the initial .rb (url + sha256 from the release tarball, license, build/install stanza), committing/PRing it to the tap; only bump on subsequent releases. WORKAROUND USED: hand-created Formula/ossctl.rb in jarimustonen/homebrew-ossctl (source-build formula) and verified 'brew install jarimustonen/ossctl/ossctl' works. Future cuts can now bump-formula-pr it.
