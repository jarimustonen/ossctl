//! Stable, per-target journal ids for a release plan's targets.
//!
//! The event-sourced journal (ADR-0003) keys every per-target fact —
//! `dry_run` / `built` / `published` — by a short string id, and the coordinator,
//! the resume reconciler, and the CLI's `RunCreated.targets` list must all derive
//! that id **identically** or a resume looks up the wrong key (and, worst case,
//! re-publishes an already-landed target). This module is the one place the id is
//! derived, so those three callers cannot drift.
//!
//! ## Why not just the ecosystem string
//!
//! Historically a cut carried at most one target per ecosystem (the normalizer
//! expanded `ecosystems` 1:1), so the ecosystem wire string (`rust`, `node`, …)
//! was itself a unique key. A contract may now declare **several** targets in one
//! ecosystem — e.g. `ossctl`'s own two crates.io crates (`ossctl-core` then
//! `ossctl`), plus a `gh-releases` and a `homebrew` target all under `rust` — so
//! the ecosystem alone collides. [`journal_target_ids`] disambiguates only as far
//! as it must: a lone-in-its-ecosystem target keeps the bare ecosystem id (so
//! single-target cuts, and every existing journal, are byte-for-byte unchanged);
//! an ecosystem with several targets qualifies each with the least of
//! `package` → `package:registry` → `package:registry:adapter` that makes the
//! group's ids distinct.
//!
//! ## Determinism (and its coupling to `plan_id`)
//!
//! The ids are a pure function of the target list (which is itself the
//! normalizer's canonical order), computed through a `BTreeMap` group scan — no
//! wall-clock, no `HashMap` iteration — so the same plan always yields the same
//! ids, in the same positions. The ids are journal keys only; they are **not**
//! part of the content-addressed `plan_id` (that hashes the target *fields*), so
//! this derivation never affects plan identity or drift detection.
//!
//! That exclusion is load-bearing for resume safety, and it holds only because
//! this derivation reads **exactly** the target fields (`ecosystem`, `package`,
//! `registry`, `adapter`, and their order) that `plan_id` also seals
//! ([`crate::release::plan`]'s `SealInput`). The coordinator writes the journal
//! keyed by these ids and resume re-derives them from the (drift-checked) plan; a
//! matching `plan_id` therefore guarantees byte-identical ids, so resume looks up
//! the same receipt the cut wrote and never re-publishes a landed target. If a
//! future edit made this function read a field `plan_id` does *not* seal (or vice
//! versa), two plans could share a `plan_id` yet key their journals differently —
//! a silent re-publish hazard. Keep the two field sets in lockstep, and bump
//! [`crate::release::plan`]'s `SEAL_VERSION` if the covered fields change.
//!
//! ## Id stability across contract edits (a documented non-guarantee)
//!
//! A target's id is stable for a given plan, **not** across contract revisions.
//! Adding a *second* target to an ecosystem that previously had one flips the
//! first target's id on the next cut from the bare `"rust"` to a qualified
//! `"rust:<disc>"`. Old runs' journals keep their `"rust"` keys forever (they are
//! never rewritten); only new runs use the qualified form. Downstream consumers
//! (`release show --json`, dashboards, log queries) must therefore not assume a
//! per-target journal id is stable across contract edits.

use std::collections::{BTreeMap, BTreeSet};

use crate::protocol::plan::PlanTarget;

/// The qualification levels a same-ecosystem group is disambiguated through, in
/// increasing verbosity. `package` alone suffices for the common multi-crate case
/// (two crates.io crates); `registry` separates same-package channels
/// (`crates.io` vs `gh-releases` vs `homebrew` for one crate); `adapter` is the
/// last resort before two targets are genuinely identical.
const MAX_LEVEL: u8 = 3;

/// Assign a stable, unique journal id to each target in `targets`, returned
/// positionally aligned with the input.
///
/// A target that is the only one in its ecosystem gets the bare ecosystem wire
/// string (`"rust"`); an ecosystem carrying several targets gets each of them
/// `"<ecosystem>:<discriminator>"`, where the discriminator is the shortest of
/// `package` / `package:registry` / `package:registry:adapter` that is distinct
/// across that ecosystem's targets.
///
/// If two targets are *byte-identical* (same ecosystem, package, registry, and
/// adapter) their ids still collide even at the fullest qualification — a
/// degenerate duplicate the caller ([`crate::release::coordinator::validate_plan`])
/// rejects rather than papering over. Ids across *different* ecosystems never
/// collide (the ecosystem prefix differs), and a bare-ecosystem id never equals a
/// qualified `"<ecosystem>:…"` id.
#[must_use]
pub fn journal_target_ids(targets: &[PlanTarget]) -> Vec<String> {
    // Group target indices by ecosystem (BTreeMap keeps the scan deterministic).
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, t) in targets.iter().enumerate() {
        groups.entry(t.ecosystem.as_str()).or_default().push(i);
    }

    let mut ids = vec![String::new(); targets.len()];
    for (eco, idxs) in &groups {
        if idxs.len() == 1 {
            // Lone target: the ecosystem string is already a unique key (and keeps
            // single-target cuts identical to how they journalled before).
            ids[idxs[0]] = (*eco).to_string();
            continue;
        }
        let level = minimal_level(targets, idxs);
        for &i in idxs {
            ids[i] = format!("{eco}:{}", discriminator(&targets[i], level));
        }
    }
    ids
}

/// The least qualification level (`1..=`[`MAX_LEVEL`]) at which every target in
/// `idxs` has a distinct [`discriminator`]. Falls back to [`MAX_LEVEL`] when even
/// the fullest form collides (two identical targets) — the caller detects the
/// resulting duplicate id and refuses the plan.
fn minimal_level(targets: &[PlanTarget], idxs: &[usize]) -> u8 {
    // Inclusive of `MAX_LEVEL`: the fullest form (`package:registry:adapter`) is a
    // real candidate that separates same-package/same-registry channels that differ
    // only by adapter — it is not merely an untested fallback.
    for level in 1..=MAX_LEVEL {
        // A `BTreeSet` keeps the membership test O(log n) without introducing any
        // ordering-unstable iteration (we read only `insert`'s bool, never iterate).
        let mut seen = BTreeSet::new();
        if idxs
            .iter()
            .all(|&i| seen.insert(discriminator(&targets[i], level)))
        {
            return level;
        }
    }
    MAX_LEVEL
}

/// The `level`-deep discriminator for one target: `package`, then
/// `package:registry`, then `package:registry:adapter`. A target with no resolved
/// package name contributes an empty package segment (an unresolved target is not
/// executable anyway — the coordinator refuses it before any external action).
fn discriminator(target: &PlanTarget, level: u8) -> String {
    let package = target.package.as_deref().unwrap_or("");
    match level {
        1 => package.to_string(),
        2 => format!("{package}:{}", target.registry.as_str()),
        _ => format!(
            "{package}:{}:{}",
            target.registry.as_str(),
            target.adapter.as_str()
        ),
    }
}

#[cfg(test)]
mod tests;
