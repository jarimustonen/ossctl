---
created: 2026-08-05
updated: 2026-08-10
type: bug
status: obsolete
priority: normal
closed: 2026-08-10
---

# Homebrew tap formula bump is manual and was missed (served v0.1.0 through the entire 0.1.1 lifetime)

## Description

The homebrew-ossctl tap formula (Formula/ossctl.rb) is a SOURCE formula (builds via cargo from the tagged source tarball) whose url+sha256 must be hand-bumped after each release. It was NOT bumped for 0.1.1 — it sat at v0.1.0 from the 0.1.0 release until the 0.1.2 cut (stint #11), so 'brew install jarimustonen/ossctl/ossctl' installed 0.1.0 the whole time crates.io/GitHub had 0.1.1. Fixed forward to v0.1.2 manually during the 0.1.2 cut (tap commit e50fbe2). Real fix: automate the tap bump as part of release.yml/publish (cargo-dist can publish a homebrew formula to a tap with a tap write token, or a post-release job can PUT the formula via API), so the tap can never lag the release again. Related: publish-crates-no-auto-trigger, release-macos-hauis-coupling — the release pipeline has several manual/fragile steps the recipe claims are automatic.
