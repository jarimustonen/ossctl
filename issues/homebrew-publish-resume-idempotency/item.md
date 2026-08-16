---
created: 2026-08-05
updated: 2026-08-16
type: bug
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@release-engine-cut-cargo-dist-flow']
lane: release-hardening
---

# homebrew post-tag publish: crash window can open a duplicate PR on resume

_Source: review of release-engine-cut-cargo-dist-flow_

## Description

The post-tag dist phase (coordinator.rs dist_phase) publishes homebrew via the adapter, then journals the TargetPublished receipt (append-then-apply). If the process dies AFTER 'brew bump-formula-pr' opened a PR but BEFORE the receipt is durably appended, resume sees no receipt for the homebrew target. Homebrew's verify() is structurally Unknown, so reconcile classifies it Unverifiable (blocked) unless --allow-unverified, and with the go-ahead it ResumePublishes -> a SECOND bump-formula-pr -> a duplicate PR (or a hard error on the existing branch). crates.io has a natural double-publish guard (registry rejects dupes); homebrew does not.

Surfaced by /llm-review (openai, anthropic, deepseek all flagged it). This is the same append-then-apply crash window that exists for every publish, but homebrew lacks a natural idempotency gate and the action is now post-tag. Pre-existing in spirit (homebrew always had this), not introduced here, but raised in stakes.

Fix direction: make the homebrew adapter idempotent — before bump/create, probe the tap for an existing open PR / branch / already-updated formula for this package@version and treat it as Matches (adopt-forward a synthetic receipt) rather than opening a second PR. Use a deterministic branch name (the adapter already does: ossctl-homebrew-<name>-<version>). Do NOT let --allow-unverified authorize retrying an irreversible homebrew PR when remote state cannot be observed. Relatedto homebrew-adapter-fs-port / the deferred homebrew hardening.

## Update (tap-write leg, 2026-08-07)

The `homebrew-dist-brew-audit-fails` fix replaced `brew bump-formula-pr` with a direct tap-write
(clone → overwrite → `git push origin HEAD`). Two resume/concurrency concerns the `/llm-review` panel
raised belong here (the tap-write path fails **closed** on both today — safe, but not yet recovered):

- **Non-fast-forward push race**: a concurrent cut for another formula in the same tap, or a manual
  commit landing between the shallow clone and the push, makes `git push origin HEAD` fail
  non-fast-forward. Fails closed; no auto re-clone/rebase-retry. Fine for sequential single-formula
  cuts; revisit if a tap ever hosts fan-out cuts.
- **Stale-version resume downgrade**: resuming an OLD version's cut after a NEWER version already
  updated the tap would clone the newer formula, render the older one, see a diff, and push a
  *downgrade* as a clean fast-forward. A version/precedence guard (or compare-and-swap on the file's
  blob SHA via the GitHub Contents API) would prevent it.

Byte-compare idempotency (a re-run at the SAME version = clean no-op) IS now handled in the tap-write
path; these two are the remaining resume-safety gaps.
