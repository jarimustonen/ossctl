---
created: 2026-08-05
updated: 2026-08-10
type: bug
status: duplicate
priority: normal
closed: 2026-08-10
---

# publish-crates.yml does not auto-trigger on release (GITHUB_TOKEN recursion guard)

## Description

During the 0.1.2 cut, the GitHub Release was created by cargo-dist's release.yml using the default GITHUB_TOKEN. GitHub deliberately does NOT emit workflow-triggering events (release: published) for releases/tags created via GITHUB_TOKEN, to prevent recursive runs. So publish-crates.yml (on: release: [published]) NEVER fired automatically — crates.io was not published until a manual 'gh workflow run publish-crates.yml' (workflow_dispatch). The handoff's claim that the tag→release→publish chain is fully automatic is WRONG; 0.1.1 was evidently also hand-dispatched. Fix options: (a) publish crates from WITHIN release.yml's announce/publish step (same run, has the token), (b) trigger via a PAT (repo secret) when creating the release so the event cascades, or (c) a workflow_run trigger keyed on release.yml completion. Until fixed, every release needs a manual publish-crates dispatch after the GitHub Release exists — document it in AGENTS.md release recipe if not auto-fixed.
