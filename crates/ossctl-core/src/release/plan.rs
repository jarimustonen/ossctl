//! The sealed, content-addressed release plan — the read-only pre-image the
//! human approves (ADR-0002 §3).
//!
//! `release plan` computes and seals a `plan_id`; `release cut --plan <plan_id>`
//! executes it and refuses on repo drift. The binary never prompts: it plans
//! and exits at the approval boundary.
//!
//! ## What `plan_id` hashes (the content address)
//!
//! [`build`] derives a [`ReleasePlan`] from the already-normalized contract and
//! detected repo facts, then content-addresses it. The `plan_id` is the
//! lowercase SHA-256 hex digest of a canonical JSON pre-image (`serde_json`,
//! whose struct-field and `BTreeMap` ordering is deterministic) covering
//! **exactly**, in this fixed order:
//!
//! 1. a domain separator + `SEAL_VERSION` — so a `plan_id` can never collide
//!    with any other ossctl digest and the canonicalization format can be
//!    evolved by a deliberate `SEAL_VERSION` bump instead of silently;
//! 2. the contract-document `schema_version` (ADR-0002 lists it explicitly);
//! 3. the **full normalized contract JSON** (`contract show`'s canonical output
//!    — every defaulted field, so any config change is drift; hashing the whole
//!    contract is deliberately *fail-closed*: a cosmetic change re-requires
//!    approval rather than risk missing a substantive one);
//! 4. the git `HEAD` sha the plan was sealed against;
//! 5. the chosen release version (the human's bump — design §3.4);
//! 6. the **resolved concrete target set** — each target's ecosystem, resolved
//!    package name, registry, and adapter *identity*. Resolution overlays
//!    facts-derived package names onto the contract's (which may be `null`), so
//!    a manifest rename is detectable drift even though the contract text is
//!    unchanged;
//! 7. the phase sequence (constant per ADR-0002 §2, so it never *causes* drift
//!    within a binary, but binding it authenticates the execution shape the
//!    approver saw and makes a future phase-model change a `SEAL_VERSION` event).
//!
//! ## Coordinator seam (what the sibling consumes)
//!
//! The coordinator refuses a `release cut --plan <id>` on drift by re-deriving
//! current state and calling [`verify`]. It needs to persist only two plain
//! fields from an approved plan — `plan_id` and `version` — into its journal;
//! the approved [`ReleasePlan`] is otherwise reconstructed via [`build`] from
//! the journalled sealed inputs. The plan DTOs are therefore `Serialize`-only,
//! matching the repo-wide convention that the wire enums (`Ecosystem`/`Registry`
//! /`Adapter`) do not derive `Deserialize` (they collect-all-errors on parse).
//! The trust boundary is the *local journal*: an approved plan is one ossctl
//! itself wrote, not untrusted caller input.
//!
//! ## Out of this worker's scope (handed to the coordinator)
//!
//! - **Working-tree cleanliness.** The seal binds `HEAD`, not uncommitted
//!   changes. Enforcing a clean tree / executing from a clean checkout of the
//!   sealed commit is an *execution* guard the coordinator owns (it needs a new
//!   read-only `GitRepo` status port). Until then a dirty tree can publish code
//!   that differs from the sealed commit — an accepted, documented gap.
//!
//! **Adapter tool *versions* (accepted gap).** ADR-0002 §3 names "resolved
//! adapter identities+versions". The adapter registry (a sibling unit) is not
//! landed, so no adapter *tool version* (e.g. a pinned `cargo-dist` release) is
//! resolvable yet; today the address binds adapter **identity** (the enum). When
//! the registry lands, fold the resolved versions into the pre-image — a
//! deliberate `schema_version`-bumping change to what the address covers, never
//! a silent one.
//!
//! Determinism: no wall-clock, no id-gen, no ordering-unstable map enters the
//! pre-image — identical `(contract, facts, head, version)` always yield the
//! same `plan_id` (proven in tests).

use serde::Serialize;

use crate::contract::schema::{Contract, Ecosystem};
use crate::protocol::facts::Facts;
use crate::protocol::plan::{PlanPhase, PlanTarget, ReleasePlan};

/// Build and seal a [`ReleasePlan`] from an already-normalized `contract` and
/// detected `facts`, at git `head_sha`, for the chosen `version`.
///
/// The caller (the `ossctl-cli` handler behind `release plan`, or the release
/// coordinator re-deriving current state) is responsible for having normalized
/// the contract and gathered the facts through the same code paths behind
/// `contract show` / `facts` — this function never re-parses `OSS-RELEASE.md`
/// nor re-derives facts. `version` is treated as an opaque, already-validated
/// identifier (scheme-specific validation — semver vs a calver pattern — is the
/// contract's/skill's job, not the plan's).
#[must_use]
pub fn build(contract: &Contract, facts: &Facts, head_sha: &str, version: &str) -> ReleasePlan {
    let targets = resolve_targets(contract, facts);
    let plan_id = seal(contract, &targets, head_sha, version);
    ReleasePlan {
        plan_id,
        contract_schema_version: contract.schema_version,
        head_sha: head_sha.to_string(),
        version: version.to_string(),
        targets,
        phases: PlanPhase::SEQUENCE.to_vec(),
        // Carried from the (already-hashed) contract so the coordinator can hand
        // the Homebrew adapter its tap + license without re-reading the contract.
        // The first distribution that declares a tap — identical to the old
        // single-`Distribution` behavior. The release-engine CLI path
        // (`ensure_single_distribution`) rejects a multi-distribution monorepo
        // BEFORE reaching here, so `distributions.len() <= 1` and this `find_map`
        // never silently drops a second distribution's tap; carrying a per-package
        // tap for a true multi-tap monorepo is a deliberate follow-up.
        homebrew_tap: contract
            .distributions
            .iter()
            .find_map(|d| d.homebrew_tap.clone()),
        license: Some(contract.license.clone()),
    }
}

/// Compute the content-addressed `plan_id` for `(contract, facts, head_sha,
/// version)` **without** allocating a full [`ReleasePlan`].
///
/// The drift-check seam for the coordinator: given the plan a human approved, it
/// re-derives the *current* repo's contract + facts + `HEAD`, calls this with
/// the approved plan's sealed `version`, and compares. Prefer [`verify`], which
/// wraps this and reports *which* inputs drifted; this raw form is exposed for
/// callers that only need the digest.
#[must_use]
pub fn compute_plan_id(
    contract: &Contract,
    facts: &Facts,
    head_sha: &str,
    version: &str,
) -> String {
    let targets = resolve_targets(contract, facts);
    seal(contract, &targets, head_sha, version)
}

/// Check whether an `approved` plan still matches the **current** repo state.
///
/// The coordinator calls this before crossing into any irreversible phase of
/// `release cut --plan <plan_id>`. It re-derives the current `plan_id` from the
/// current `contract`, `facts`, and `head_sha`, holding the *chosen version*
/// fixed to the approved plan's (a cut may not change the sealed version — that
/// would require a new plan). `Ok(())` means the approval is still valid; a
/// [`PlanDrift`] carries the mismatched id pair and human-readable reasons for
/// the `plan_stale` error envelope. The `plan_id` mismatch is authoritative;
/// the reasons are **best-effort and may be non-exhaustive** — the approved
/// plan intentionally does not retain the old normalized contract (trust the
/// journal, not a re-supplied contract), so an exact field-level contract diff
/// is not possible here. When more than one input drifts, the reasons name
/// every one they can pinpoint (`HEAD`, schema version, target set) and fall
/// back to a generic contract-changed note only when none of those explain it.
///
/// # Errors
/// Returns [`PlanDrift`] when the recomputed `plan_id` differs from
/// `approved.plan_id` — i.e. the repo moved (a commit, a manifest rename, a
/// schema bump, a target-set change, or any normalized-contract change) since
/// approval.
pub fn verify(
    approved: &ReleasePlan,
    contract: &Contract,
    facts: &Facts,
    head_sha: &str,
) -> Result<(), PlanDrift> {
    let current_targets = resolve_targets(contract, facts);
    let current_id = seal(contract, &current_targets, head_sha, &approved.version);
    if current_id == approved.plan_id {
        return Ok(());
    }

    // The ids differ; pinpoint *why* so the coordinator can surface an
    // actionable `plan_stale` message rather than a bare hash mismatch.
    let mut reasons = Vec::new();
    if approved.head_sha != head_sha {
        reasons.push(format!(
            "HEAD moved from {} to {}",
            short_sha(&approved.head_sha),
            short_sha(head_sha)
        ));
    }
    if approved.contract_schema_version != contract.schema_version {
        reasons.push(format!(
            "contract schema_version changed from {} to {}",
            approved.contract_schema_version, contract.schema_version
        ));
    }
    if approved.targets != current_targets {
        reasons.push(
            "the resolved target set changed (a target, package, registry, or adapter differs)"
                .to_string(),
        );
    }
    // A change the specific probes above did not catch (any other normalized
    // contract field: version scheme, changelog, license, health badges, …).
    if reasons.is_empty() {
        reasons.push("the normalized contract changed".to_string());
    }

    Err(PlanDrift {
        approved_plan_id: approved.plan_id.clone(),
        current_plan_id: current_id,
        reasons,
    })
}

/// Why a `release cut --plan <plan_id>` was refused: the current repo no longer
/// hashes to the approved plan (ADR-0002 §3, `plan_stale`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanDrift {
    /// The `plan_id` the human approved.
    pub approved_plan_id: String,
    /// The `plan_id` the current repo state produces.
    pub current_plan_id: String,
    /// Human-readable specifics of what drifted (`HEAD` moved, the target set
    /// changed, …) — at least one entry.
    pub reasons: Vec<String>,
}

/// A publishable target whose **tree manifest** declares a version different
/// from the version a cut would seal — the drift the `--version` guard rejects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionMismatch {
    /// The resolved package whose manifest version disagrees with `--version`.
    pub package: String,
    /// The package's ecosystem.
    pub ecosystem: Ecosystem,
    /// The version declared in the tree manifest — what the ecosystem's publish
    /// command (`cargo publish` reading `Cargo.toml`, …) would **actually**
    /// upload, regardless of the chosen `--version`.
    pub manifest_version: String,
}

/// Reject a chosen `version` that does not match the tree manifest version(s) a
/// cut would actually publish.
///
/// `ossctl release cut` does **not** bump the manifest: each ecosystem's publish
/// command uploads the version already in the tree (`cargo publish` reads
/// `Cargo.toml`), while the engine threads the chosen `--version` into every
/// registry probe, index-wait, and receipt (a plan target's
/// [`version`](crate::protocol::plan::ReleasePlan::version) is the sealed
/// `--version`, not the manifest's). When the two disagree the engine publishes
/// the *manifest* version yet waits for — and records — the requested version,
/// which never lands, so the cut fails obscurely (an index-visibility timeout on
/// a dependency that was published at a different version) and nothing reaches
/// the registry at `version` (`release-cut-publish-noop`).
///
/// This guard turns that silent footgun into a fast, pre-publish `plan`/`cut`
/// refusal: for every target whose resolved package declares a version in
/// `facts`, that version must equal `version`. Targets whose package has no
/// detectable manifest version — a homebrew/binary distribution target, an
/// undetected package — are not checked (there is no tree version to compare,
/// and their release version is bound to the crate they repackage, not to a
/// manifest of their own).
///
/// # Errors
/// Returns every [`VersionMismatch`] found (sorted by package, one per package)
/// when at least one checkable target's manifest version differs from `version`;
/// `Ok(())` when all checkable targets agree.
pub fn check_version_matches_tree(
    contract: &Contract,
    facts: &Facts,
    version: &str,
) -> Result<(), Vec<VersionMismatch>> {
    let mut mismatches: Vec<VersionMismatch> = Vec::new();
    for t in resolve_targets(contract, facts) {
        let Some(package) = t.package else { continue };
        // The tree manifest version for this resolved (ecosystem, package). Absent
        // ⇒ nothing to compare (a distribution target, or an undetected manifest).
        let Some(manifest_version) = facts
            .packages
            .iter()
            .find(|p| p.ecosystem == t.ecosystem && p.package.as_deref() == Some(package.as_str()))
            .and_then(|p| p.version.clone())
        else {
            continue;
        };
        if manifest_version != version {
            mismatches.push(VersionMismatch {
                package,
                ecosystem: t.ecosystem,
                manifest_version,
            });
        }
    }
    // Deterministic order, and one row per package even if a package backs several
    // targets (a crate published to crates.io AND repackaged for homebrew). Sort and
    // dedup on the SAME (ecosystem, package) key so equal keys are guaranteed adjacent
    // before the consecutive-only `dedup_by` runs.
    mismatches.sort_by(|a, b| {
        (a.ecosystem.as_str(), &a.package).cmp(&(b.ecosystem.as_str(), &b.package))
    });
    mismatches.dedup_by(|a, b| a.package == b.package && a.ecosystem == b.ecosystem);
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(mismatches)
    }
}

/// Overlay facts-derived package names onto the contract's target set, yielding
/// the concrete targets a cut would execute. Order follows the contract's
/// `targets` (already canonicalized by the normalizer).
fn resolve_targets(contract: &Contract, facts: &Facts) -> Vec<PlanTarget> {
    contract
        .targets
        .iter()
        .map(|t| {
            let package = t
                .package
                .clone()
                .or_else(|| resolve_package(facts, t.ecosystem));
            PlanTarget {
                ecosystem: t.ecosystem,
                package,
                registry: t.registry,
                adapter: t.adapter,
            }
        })
        .collect()
}

/// The detected package name for `ecosystem`, resolved **only when
/// unambiguous** — exactly one named manifest for that ecosystem.
///
/// `None` when no manifest named one (a virtual workspace, a binary-only repo)
/// **or** when several do (a monorepo with multiple crates of one ecosystem):
/// with no per-target manifest key in the contract, picking the first would
/// silently mis-assign the same package to every `null` target, so we leave it
/// `null` for cut-time inference instead. A monorepo should declare explicit
/// per-target `package`s in the contract; the CLI warns when this fires.
fn resolve_package(facts: &Facts, ecosystem: crate::contract::schema::Ecosystem) -> Option<String> {
    let mut named = facts
        .packages
        .iter()
        .filter(|p| p.ecosystem == ecosystem && p.package.is_some());
    let first = named.next()?;
    // More than one named candidate ⇒ ambiguous ⇒ do not guess.
    if named.next().is_some() {
        return None;
    }
    first.package.clone()
}

/// Domain separator baked into every pre-image so a `plan_id` can never be
/// confused with any other SHA-256 an ossctl subsystem might compute over
/// similar bytes. Ends in the seal-format version for readability; the numeric
/// [`SEAL_VERSION`] is also hashed as its own field.
const SEAL_DOMAIN: &str = "ossctl.release-plan";

/// Version of the *hashing pre-image format* — the field set, their order, and
/// the canonicalization. Independent of the contract-document or wire-envelope
/// versions. Bump this (never silently) whenever the pre-image shape changes
/// (e.g. once resolved adapter versions are folded in), so old and new plan ids
/// are intentionally disjoint rather than accidentally colliding.
const SEAL_VERSION: u32 = 5;

/// The canonical hashed pre-image (see the module docs for the exact contents).
/// A dedicated struct rather than an ad-hoc byte concatenation so the field set
/// is explicit and serde's deterministic struct-field ordering fixes the byte
/// layout.
///
/// **DO NOT REORDER these fields** — field order is part of the content address,
/// so a reorder silently changes every `plan_id`. Evolve the format via
/// [`SEAL_VERSION`] instead.
#[derive(Serialize)]
struct SealInput<'a> {
    domain: &'static str,
    seal_version: u32,
    contract_schema_version: u32,
    contract: &'a Contract,
    head_sha: &'a str,
    version: &'a str,
    targets: &'a [PlanTarget],
    phases: &'a [PlanPhase],
}

/// Serialize the pre-image to canonical JSON and return its SHA-256 hex digest.
fn seal(contract: &Contract, targets: &[PlanTarget], head_sha: &str, version: &str) -> String {
    let input = SealInput {
        domain: SEAL_DOMAIN,
        seal_version: SEAL_VERSION,
        contract_schema_version: contract.schema_version,
        contract,
        head_sha,
        version,
        targets,
        phases: &PlanPhase::SEQUENCE,
    };
    // `to_vec` on a struct of only structs/Vecs/BTreeMaps (contract's
    // `extra_fields` is a `serde_json::Map` = `BTreeMap` without the
    // `preserve_order` feature) is deterministic — no wall-clock, no HashMap,
    // no float. It is also infallible for these concrete types; `expect` (never
    // `unwrap_or_default`, which would fail *open* by hashing an empty pre-image
    // and collide every failing plan on the empty-string digest).
    let bytes =
        serde_json::to_vec(&input).expect("release-plan pre-image is infallible to serialize");
    sha256::hex(&bytes)
}

/// Short (first 12 hex chars) `HEAD` sha for drift messages; whole string if
/// shorter.
fn short_sha(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// A self-contained SHA-256 (FIPS 180-4) so `plan_id` needs no third-party hash
/// dependency and no edit to the workspace `Cargo.toml` (a hot file). Content
/// addressing is an integrity check over local, non-adversarial inputs, so a
/// vendored reference implementation is appropriate; correctness is pinned by
/// the RFC known-answer vectors in the module tests.
mod sha256 {
    // The canonical reference form is dense in bit-twiddling and single-letter
    // working variables; the lints below fight that idiom for no clarity gain.
    #![allow(
        clippy::unreadable_literal,
        clippy::many_single_char_names,
        clippy::needless_range_loop
    )]

    use std::fmt::Write as _;

    /// SHA-256 round constants (first 32 bits of the fractional parts of the
    /// cube roots of the first 64 primes).
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    /// Initial hash values (first 32 bits of the fractional parts of the square
    /// roots of the first 8 primes).
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    /// The lowercase 64-character SHA-256 hex digest of `data`.
    pub fn hex(data: &[u8]) -> String {
        let mut h = H0;

        // Pad: 0x80, then zeros to a 56-mod-64 boundary, then the 64-bit
        // big-endian bit length.
        let mut msg = data.to_vec();
        // FIPS 180-4 caps the message at 2^64 - 1 bits; a checked multiply turns
        // the (practically unreachable) overflow into a loud panic rather than a
        // silently wrong digest.
        let bit_len = (data.len() as u64)
            .checked_mul(8)
            .expect("SHA-256 input exceeds 2^64 bits");
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in msg.chunks_exact(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[4 * i],
                    chunk[4 * i + 1],
                    chunk[4 * i + 2],
                    chunk[4 * i + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let mut a = h[0];
            let mut b = h[1];
            let mut c = h[2];
            let mut d = h[3];
            let mut e = h[4];
            let mut f = h[5];
            let mut g = h[6];
            let mut hh = h[7];

            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }

            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }

        let mut out = String::with_capacity(64);
        for v in h {
            let _ = write!(out, "{v:08x}");
        }
        out
    }
}

#[cfg(test)]
mod tests;
