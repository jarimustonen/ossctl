//! Immutable, content-addressed storage for approved release plans (ADR-0003).
//!
//! Plan documents live beside release journals under `ossctl/plans`. The document
//! retains both the public plan and the exact canonical seal pre-image, allowing a
//! later cut or resume to authenticate it without consulting a changed worktree.

use std::fs;
use std::io;

use serde::Serialize;
use serde_json::Value;

use crate::contract::schema::Contract;
use crate::contract::schema::{Adapter, Ecosystem, Registry};
use crate::protocol::plan::{BumpLevel, BumpPlan, PinRewrite, PlanPhase, PlanTarget, ReleasePlan};
use crate::release::journal::JournalPaths;
use crate::release::plan::{seal_bytes, seal_id_from_bytes};

/// A plan-store failure. Corruption has a stable discriminator so CLI callers never
/// mistake a damaged local approval artifact for a missing legacy plan.
#[derive(Debug)]
pub enum PlanStoreError {
    /// Filesystem access failed.
    Io(io::Error),
    /// A stored document fails its content-address integrity check.
    Corrupt {
        /// Address requested by the caller.
        plan_id: String,
        /// Specific malformed or mismatching field.
        detail: String,
    },
    /// An existing address contains bytes different from a retry's document.
    ContentAddressViolation {
        /// Address whose immutable content was contradicted.
        plan_id: String,
    },
}

impl std::fmt::Display for PlanStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Corrupt { plan_id, detail } => {
                write!(f, "plan_store_corrupt: {plan_id}: {detail}")
            }
            Self::ContentAddressViolation { plan_id } => write!(
                f,
                "plan store already contains different content for {plan_id}"
            ),
        }
    }
}
impl std::error::Error for PlanStoreError {}
impl From<io::Error> for PlanStoreError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Result of discarding a sealed plan from the durable store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardOutcome {
    /// The authenticated plan document was removed.
    Discarded,
    /// A durable disposal marker proves an earlier request removed the plan.
    AlreadyDiscarded,
    /// Neither a plan nor a disposal marker has ever existed at this address.
    Unknown,
}

#[derive(Serialize)]
struct StoredPlan<'a> {
    plan: &'a ReleasePlan,
    seal_preimage: String,
}

/// Persist and authenticate sealed plans at paths derived from [`JournalPaths`].
pub struct PlanStore {
    paths: JournalPaths,
}
impl PlanStore {
    /// Create a store rooted beside `paths`' release-journal root.
    #[must_use]
    pub fn new(paths: JournalPaths) -> Self {
        Self { paths }
    }

    /// Create a document if absent. A same-byte retry is a no-op; any other
    /// content under the same address is an integrity violation.
    pub fn save(&self, plan: &ReleasePlan, contract: &Contract) -> Result<(), PlanStoreError> {
        let preimage = seal_bytes(
            contract,
            &plan.targets,
            &plan.head_sha,
            &plan.version,
            &plan.phases,
            plan.bump.as_ref(),
        );
        let bytes = serde_json::to_vec(&StoredPlan {
            plan,
            seal_preimage: String::from_utf8(preimage).expect("canonical JSON is UTF-8"),
        })
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let path = self.paths.plan_file(&plan.plan_id);
        match fs::read(&path) {
            Ok(existing) if existing == bytes => {
                self.clear_discard_marker(&plan.plan_id)?;
                return Ok(());
            }
            Ok(_) => {
                return Err(PlanStoreError::ContentAddressViolation {
                    plan_id: plan.plan_id.clone(),
                })
            }
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            Err(_) => {}
        }
        fs::create_dir_all(self.paths.plans_dir())?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&tmp, &bytes)?;
        // Do not replace a concurrent writer: inspect again immediately before rename.
        match fs::hard_link(&tmp, &path) {
            Ok(()) => {
                fs::remove_file(tmp)?;
                self.clear_discard_marker(&plan.plan_id)?;
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                fs::remove_file(tmp)?;
                if fs::read(&path)? == bytes {
                    self.clear_discard_marker(&plan.plan_id)?;
                    Ok(())
                } else {
                    Err(PlanStoreError::ContentAddressViolation {
                        plan_id: plan.plan_id.clone(),
                    })
                }
            }
            Err(e) => {
                let _ = fs::remove_file(tmp);
                Err(e.into())
            }
        }
    }

    /// Load and authenticate a plan. Missing plans are the compatibility path for
    /// plans made by older binaries or on another machine.
    pub fn load(&self, plan_id: &str) -> Result<Option<ReleasePlan>, PlanStoreError> {
        let path = self.paths.plan_file(plan_id);
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let doc: Value =
            serde_json::from_slice(&bytes).map_err(|e| corrupt(plan_id, e.to_string()))?;
        let preimage = doc
            .get("seal_preimage")
            .and_then(Value::as_str)
            .ok_or_else(|| corrupt(plan_id, "missing seal_preimage"))?;
        if seal_id_from_bytes(preimage.as_bytes()) != plan_id {
            return Err(corrupt(plan_id, "seal hash does not match filename"));
        }
        let plan = decode_plan(
            doc.get("plan")
                .ok_or_else(|| corrupt(plan_id, "missing plan"))?,
            plan_id,
        )?;
        if plan.plan_id != plan_id {
            return Err(corrupt(plan_id, "plan_id does not match filename"));
        }
        Ok(Some(plan))
    }

    /// Authenticate and remove a sealed plan document.
    ///
    /// A durable marker distinguishes an idempotent retry from a well-formed but
    /// genuinely unknown address. A present document is fully authenticated before
    /// deletion, so corruption is never erased under the guise of disposal. Callers
    /// coordinating this with release-run creation must hold the repository's
    /// single-active-cut lock.
    pub fn discard(&self, plan_id: &str) -> Result<DiscardOutcome, PlanStoreError> {
        if !is_plan_id(plan_id) {
            return Err(PlanStoreError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "invalid plan id {plan_id:?}: expected 64 lowercase hexadecimal characters"
                ),
            )));
        }
        if self.load(plan_id)?.is_none() {
            return Ok(if self.paths.discarded_plan_file(plan_id).is_file() {
                DiscardOutcome::AlreadyDiscarded
            } else {
                DiscardOutcome::Unknown
            });
        }

        self.write_discard_marker(plan_id)?;
        let path = self.paths.plan_file(plan_id);
        match fs::remove_file(&path) {
            Ok(()) => {
                sync_dir(&self.paths.plans_dir())?;
                Ok(DiscardOutcome::Discarded)
            }
            // A concurrent idempotent retry may have won after our authenticated
            // load. Under the release lock this is not expected, but remains safe.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(DiscardOutcome::AlreadyDiscarded)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn write_discard_marker(&self, plan_id: &str) -> Result<(), PlanStoreError> {
        let marker = self.paths.discarded_plan_file(plan_id);
        let parent = marker.parent().expect("discard marker has a parent");
        fs::create_dir_all(parent)?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(file) => file.sync_all()?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        sync_dir(parent)?;
        Ok(())
    }

    fn clear_discard_marker(&self, plan_id: &str) -> Result<(), PlanStoreError> {
        let marker = self.paths.discarded_plan_file(plan_id);
        match fs::remove_file(&marker) {
            Ok(()) => sync_dir(marker.parent().expect("discard marker has a parent"))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}

fn sync_dir(path: &std::path::Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

/// Whether `value` is a canonical SHA-256 plan address.
#[must_use]
pub fn is_plan_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn corrupt(id: &str, detail: impl Into<String>) -> PlanStoreError {
    PlanStoreError::Corrupt {
        plan_id: id.to_string(),
        detail: detail.into(),
    }
}
fn str_at<'a>(v: &'a Value, key: &str, id: &str) -> Result<&'a str, PlanStoreError> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt(id, format!("missing or invalid {key}")))
}
fn decode_plan(v: &Value, id: &str) -> Result<ReleasePlan, PlanStoreError> {
    let targets = v
        .get("targets")
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt(id, "invalid targets"))?
        .iter()
        .map(|t| {
            Ok(PlanTarget {
                ecosystem: Ecosystem::parse(str_at(t, "ecosystem", id)?)
                    .ok_or_else(|| corrupt(id, "invalid ecosystem"))?,
                package: t.get("package").and_then(Value::as_str).map(str::to_string),
                registry: Registry::parse(str_at(t, "registry", id)?)
                    .ok_or_else(|| corrupt(id, "invalid registry"))?,
                adapter: Adapter::parse(str_at(t, "adapter", id)?)
                    .ok_or_else(|| corrupt(id, "invalid adapter"))?,
            })
        })
        .collect::<Result<Vec<_>, PlanStoreError>>()?;
    let phases = v
        .get("phases")
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt(id, "invalid phases"))?
        .iter()
        .map(|p| match p.as_str() {
            Some("bump") => Ok(PlanPhase::Bump),
            Some("dry-run-all") => Ok(PlanPhase::DryRunAll),
            Some("build-all") => Ok(PlanPhase::BuildAll),
            Some("publish-all") => Ok(PlanPhase::PublishAll),
            Some("tag") => Ok(PlanPhase::Tag),
            Some("dist") => Ok(PlanPhase::Dist),
            Some("verify") => Ok(PlanPhase::Verify),
            _ => Err(corrupt(id, "invalid phase")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bump = match v.get("bump") {
        None | Some(Value::Null) => None,
        Some(b) => Some(BumpPlan {
            level: BumpLevel::parse(str_at(b, "level", id)?)
                .ok_or_else(|| corrupt(id, "invalid bump level"))?,
            from_version: str_at(b, "from_version", id)?.into(),
            to_version: str_at(b, "to_version", id)?.into(),
            pin_rewrites: b
                .get("pin_rewrites")
                .and_then(Value::as_array)
                .ok_or_else(|| corrupt(id, "invalid pin_rewrites"))?
                .iter()
                .map(|p| {
                    Ok(PinRewrite {
                        in_package: str_at(p, "in_package", id)?.into(),
                        dependency: str_at(p, "dependency", id)?.into(),
                        from: str_at(p, "from", id)?.into(),
                        to: str_at(p, "to", id)?.into(),
                    })
                })
                .collect::<Result<Vec<_>, PlanStoreError>>()?,
            changelog_finalize: b
                .get("changelog_finalize")
                .and_then(Value::as_bool)
                .ok_or_else(|| corrupt(id, "invalid changelog_finalize"))?,
            bump_hook: b
                .get("bump_hook")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
    };
    Ok(ReleasePlan {
        plan_id: str_at(v, "plan_id", id)?.into(),
        contract_schema_version: u32::try_from(
            v.get("contract_schema_version")
                .and_then(Value::as_u64)
                .ok_or_else(|| corrupt(id, "invalid contract_schema_version"))?,
        )
        .map_err(|_| corrupt(id, "contract_schema_version exceeds u32"))?,
        head_sha: str_at(v, "head_sha", id)?.into(),
        version: str_at(v, "version", id)?.into(),
        targets,
        phases,
        bump,
        homebrew_tap: v
            .get("homebrew_tap")
            .and_then(Value::as_str)
            .map(str::to_string),
        license: v.get("license").and_then(Value::as_str).map(str::to_string),
        description: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        homebrew_platforms: v
            .get("homebrew_platforms")
            .and_then(Value::as_array)
            .ok_or_else(|| corrupt(id, "invalid homebrew_platforms"))?
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| corrupt(id, "invalid platform"))
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_v5_plan_without_verify_remains_readable() {
        // Existing v5 plans must load so an interrupted run can resume through the
        // now-mandatory verify barrier. A fresh cut re-derives a v7 address and
        // rejects this old approval as stale instead of silently extending it.
        let plan = serde_json::json!({
            "plan_id": "legacy",
            "contract_schema_version": 1,
            "head_sha": "abc",
            "version": "1.0.0",
            "targets": [],
            "phases": ["dry-run-all", "build-all", "publish-all", "tag", "dist"],
            "homebrew_tap": null,
            "license": null,
            "description": null,
            "homebrew_platforms": []
        });

        let decoded = decode_plan(&plan, "legacy").expect("legacy plan is valid");
        assert_eq!(decoded.phases, PlanPhase::SEQUENCE[..5]);
    }

    #[test]
    fn legacy_v6_bump_plan_remains_readable_after_pin_set_semantics_change() {
        let plan = serde_json::json!({
            "plan_id": "legacy-v6",
            "contract_schema_version": 4,
            "head_sha": "abc",
            "version": "0.5.0",
            "targets": [],
            "phases": ["bump", "dry-run-all", "build-all", "publish-all", "tag", "dist", "verify"],
            "bump": {
                "level": "minor",
                "from_version": "0.4.0",
                "to_version": "0.5.0",
                "pin_rewrites": [{
                    "in_package": "cli",
                    "dependency": "core",
                    "from": "=0.4.0",
                    "to": "=0.5.0"
                }],
                "changelog_finalize": true
            },
            "homebrew_tap": null,
            "license": null,
            "description": null,
            "homebrew_platforms": []
        });

        let decoded = decode_plan(&plan, "legacy-v6").expect("v6 bump plan remains loadable");
        assert_eq!(decoded.bump.unwrap().pin_rewrites.len(), 1);
        assert_eq!(decoded.phases.last(), Some(&PlanPhase::Verify));
    }
}
