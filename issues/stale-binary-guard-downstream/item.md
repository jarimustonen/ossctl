---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: in-progress
priority: high
related: ['@release-cut-stale-binary-guard']
lane: release-safety
lane_seq: 1
---

# Stale-binary guard blocks every downstream release plan/cut

## Description

## The regression

The stale-binary guard landed today (`release-cut-stale-binary-guard`, commit `a2f3d59`)
compares the running binary's compiled commit against the **release tree's** `HEAD`. For
ossctl's own self-cut that is exactly right. For **every other repository it is always wrong**:
ossctl is a generic release tool whose whole purpose is cutting *downstream* projects, and a
downstream tree's HEAD has no relationship to the commit ossctl was built from. The two can
never match.

Reproduced 2026-08-16 with a binary built from `7b36785`:

```
$ cd <a downstream repo> && ossctl release plan --json
{"error":{"code":"stale_binary","message":"STALE BINARY: this ossctl executable was built
from commit 7b36785…, but the release tree is at c479afd…. Rebuild the current tree with
`cargo build --release -p ossctl` before planning or cutting a release"}}
```

The suggested remedy is also wrong in that context — rebuilding "the current tree" means
rebuilding the *downstream project*, which does not produce an ossctl binary at all.

## Why this is worse than a nuisance

`--allow-stale-binary` makes it pass, so the practical outcome is that every downstream user
learns to pass the escape hatch on every invocation. Once that habit forms, the guard no longer
protects ossctl's own cut either — which was its entire reason for existing. A guard that must
be routinely bypassed is worse than no guard, because it also carries false authority.

## Direction

The check is meaningful only when the release tree **is** ossctl's own source tree — the
dogfood/self-cut case. Detect that (the tree being planned is the tree this binary was built
from) and apply the guard there; otherwise skip it entirely, silently. If some cross-tree signal
is still wanted, "the binary is older than the newest release of itself" is a different and
weaker question, and should not block a plan.

Whatever is chosen, the acceptance below is what matters: cutting a downstream project must work
with no escape hatch, and cutting ossctl from a stale ossctl binary must still refuse.

## Acceptance

- `release plan` / `release cut` in a downstream repository succeed with no `--allow-stale-binary`
  and no warning about the tree's commit.
- `release plan` / `release cut` in ossctl's own tree still refuse when the binary was built from
  a different commit than that tree.
- Regression coverage for both, so the self-cut guard cannot be silently traded away for the
  downstream fix or vice versa.
- The error text, where it still fires, names an action that makes sense in the context it fires
  in.

## Note on how this was found

Not by review or by test — by a downstream agent hitting it while trying to cut a real release,
within hours of the guard shipping. Worth recording as evidence for the repository's standing
question of which findings deserve work: this class (a shipped change breaking the tool's primary
use case, reported from the field) is the opposite end of the scale from the speculative
hardening pruned earlier the same day.
