---
created: 2026-08-17
updated: 2026-08-17
type: bug
reporter: jari
status: fixed
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
closed: 2026-08-17
closed_by: agent-stint-23
---

# verify phase races CI-delegated targets and fails a successful release

## Description

verify phase races CI-delegated targets and fails a successful release

`release cut`'s new `verify` phase runs immediately after `tag`, without waiting for a target
whose publish it just delegated to CI. cargo-dist needs minutes to build and upload, so verify
observes an empty GitHub Release and fails a release that in fact succeeds.

Observed (project-canon v0.3.3, ossctl 0.6.1, 2026-08-17):

    → publish
      published: rust:project-canon-core:crates.io@0.3.3
      published: rust:project-canon-cli:crates.io@0.3.3
      delegated to CI: rust:project-canon-cli:gh-releases (cargo-dist)   ← async, not done
    ✓ publish complete
    → tag
      tag pushed: v0.3.3
      release delegated to CI: v0.3.3 (cargo-dist)
    ✓ tag complete
    → dist
    ✓ dist complete
    → verify
      verified: rust:project-canon-core:crates.io (matches)
      verified: rust:project-canon-cli:crates.io (matches)
      verified: rust:project-canon-cli:gh-releases (missing)
    ✗ verify failed

    {"error":{"code":"release_failed","message":"… verify-phase failed on target
     `rust:project-canon-cli:gh-releases`: … is missing at its destination. Nothing was rolled
     back … inspect the journal and reconcile the registries manually before retrying"}}

The GitHub Actions run for that same tag completed **successfully 3m36s later**, publishing all
expected assets (3 platform tarballs + sha256s, installer, formula, dist-manifest), and the
Homebrew tap formula updated correctly. So the release was wholly successful; only verify's
timing was wrong.

Expected, in rough order of preference:

1. For a target the run itself marked `delegated to CI`, verify should **poll with a timeout**
   until the artifacts appear (the run knows the tag and the expected asset set from
   `dist-manifest`), rather than sampling once.
2. Failing that, such a target should be reported as `pending (delegated to CI)` and excluded
   from the pass/fail verdict, with guidance to re-run `release verify <run-id>` later — which
   already exists and is the right tool.
3. At minimum the error text must not read as registry corruption. "Missing at its destination
   … reconcile the registries manually before retrying" describes a partial-publish incident;
   here nothing was wrong. Distinguish "not there yet" from "not there".

Why it matters beyond cosmetics: this is precisely the point where an operator or agent is
primed to intervene. The stated recovery ("reconcile the registries manually before retrying")
invites either hand-editing a registry or cutting an unnecessary follow-up version — both worse
than doing nothing, and both irreversible on crates.io. In this session the temptation was
concrete: the preceding two releases had genuinely failed to attach artifacts, so a third
failure signal was highly plausible. It was only disproved by checking GitHub Actions directly.

Note the verify phase itself is a **real improvement** and caught nothing false about crates.io.
The v0.2.0/v0.3.0 cuts of this same repo reported complete success while attaching zero binaries
— exactly what verify exists to prevent. This report is about the async-target case only.

## Resolution

### 2026-08-17T17:10:11Z · @agent-stint-23

Already fixed before this report was triaged; no code change needed.

REASON. The verify barrier does not race cargo-dist. `coordinator.rs` defines DELEGATED_RELEASE_VERIFY_TIMEOUT_SECS = 20 min with a 15 s poll interval specifically for delegated GitHub Releases, so verify polls for the CI-uploaded assets rather than observing once and failing. That wait shipped in 0.7.0 (commit ba4769d, tag v0.7.0), i.e. after the 0.6.1 the report names.

EVIDENCE. ossctl's own 0.8.0 engine cut (run 01M07F9ATMGBMSNSDKRV859XZE, 2026-08-17) verified `rust:ossctl:gh-releases (matches)` on exactly this delegated path, and the tag CI produced all 11 release assets.

CAVEAT ON THE REPORT. The transcript is labelled ossctl 0.6.1, which predates the verify phase entirely — 0.6.1 should not have printed a verify barrier at all. The most likely explanation is a locally-built development binary carrying a stale version string. Maintainer confirmed the disposition (2026-08-17).

REOPEN CONDITION. Reopen if a verify phase fails with `gh-releases (missing)` on a binary that self-reports >= 0.7.0 and whose cargo-dist CI later completed successfully within the 20-minute window. That would mean the poll is not being entered on some contract shape, and the next step would be to capture the journal for that run.
