---
created: 2026-08-16
updated: 2026-08-16
type: feature
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
---

# release plan/cut should cover the Homebrew tap leg

## Description

release plan/cut should cover the Homebrew tap leg

`ossctl release plan` seals only the crates.io targets, so the Homebrew leg is left to the
human/agent to do by hand after every release.

Observed (project-canon, 2026-08-16, ossctl 0.2.x — two releases the same day):

    $ ossctl release plan --version 0.3.0
    targets:    2
      rust     crates.io    cargo-publish        (package: project-canon-core)
      rust     crates.io    cargo-publish        (package: project-canon-cli)
    phases:     dry-run-all → build-all → publish-all → tag → dist

`ossctl release cut` then ran all five phases green and reported "release complete —
published 2 target(s)". The tap was untouched, so for BOTH v0.2.0 and v0.3.0 the formula had
to be updated by hand:

    curl -sL -o v.tar.gz https://github.com/<owner>/<repo>/archive/refs/tags/v<ver>.tar.gz
    shasum -a 256 v.tar.gz
    # edit Formula/<name>.rb: bump url + sha256
    git commit && git push

Expected: the tap is a declared publish target in the contract
(`homebrew_tap: <owner>/homebrew-<name>` is present in `OSS-RELEASE.md` and
`ossctl contract show` reports 2 targets while the project has 3 publish destinations), so
`release plan` should seal a Homebrew target too and `cut` should execute it — fetch the tag
tarball, compute the sha256, rewrite `url` + `sha256` in the formula, commit and push to the
tap repo. Ordering: after `tag` (the tarball must exist) and idempotent on re-run.

Why it matters: a source-build formula's sha256 is derived mechanically from a tag the engine
just created, so there is no judgement in the step — but it is silently omitted, and "release
complete" reads as if every channel shipped. A release cut by an agent that trusts that
message leaves users on the previous version via `brew`. This was noted as a follow-up in the
project-canon handoff from v0.1.1 onward and has now cost manual work on three releases.

Note: cargo-dist can push a formula itself, so an alternative shape is for the `dist` phase to
own the tap. Either way the engine, not the human, should close the loop.
