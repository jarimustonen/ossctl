//! Public wire DTOs for the event-sourced release journal (ADR-0003).
//!
//! Two durable representations ride on these types:
//!
//! - **`journal.jsonl`** — the append-only event log: one [`JournalEvent`] per
//!   line, each self-contained (it carries its own [`JOURNAL_SCHEMA_VERSION`], a
//!   monotonic [`JournalEvent::seq`], a [`JournalEvent::ts`], an
//!   [`JournalEvent::idempotency_key`], and its [`EventKind`] payload). The log
//!   is the **single source of truth**.
//! - **`manifest.json`** — the materialized [`RunState`] projection reduced from
//!   the log. It is **disposable and reconstructable** from the events (ADR-0003
//!   §2): if the manifest cannot be rebuilt from the journal there would be two
//!   sources of truth, which is forbidden. It exists only as an O(1) cache for
//!   `release show`.
//!
//! ## Versioning + forward tolerance
//!
//! The journal is durable across `ossctl` upgrades — a run started under one
//! version must be resumable under the next — so each event carries its **own**
//! [`JOURNAL_SCHEMA_VERSION`], independent of the envelope [`crate::SCHEMA_VERSION`]
//! that versions the `--json` wire surface. Additive fields are tolerated (serde
//! ignores unknown fields on read); a *newer required* event schema is refused
//! with an actionable error rather than silently mutating state (the refusal
//! lives in [`crate::release::journal`], which reads these back).
//!
//! These DTOs are **owned by the journal**: siblings (the plan model, the
//! adapters) may hold richer in-memory receipt types, but what the journal
//! *persists* is exactly the shape here.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Schema version stamped on every [`JournalEvent`] and [`RunState`].
///
/// Monotonic integer, **independent** of [`crate::SCHEMA_VERSION`]: that one
/// versions the transient `--json`/`--output=jsonl` wire surface; this one
/// versions the *durable* on-disk journal, which must survive `ossctl` upgrades.
/// Bump on a breaking event/state change (removing/renaming fields, changing a
/// variant's semantics); additive optional fields do not bump it.
///
/// **v2** (2026-08-05): added the [`EventKind::TargetDelegated`] event class and
/// the post-tag [`Phase::Dist`] barrier (a new event *kind* / phase value an
/// older reader cannot interpret, so the version is bumped per the migration
/// rule — a v1 `ossctl` refuses a v2 line rather than misreading it). A v2 run
/// carries no v1-incompatible receipt shape; the reduce path stays
/// backward-tolerant of v1 logs (which simply lack these events).
///
/// **v3** (2026-08-05): added the [`EventKind::GithubReleaseDelegated`] event class
/// (the coordinator delegating GitHub Release creation to a target's CI, e.g.
/// `cargo-dist`). This is a **new event kind a v2 reader cannot interpret**, so the
/// migration rule requires its own bump — folding it into v2 would defeat the
/// version gate ([`read_events`](crate::release::journal::read_events)), letting a
/// v2 binary silently choke on a `github_release_delegated` line instead of refusing
/// it with an upgrade error. (This matters even though the engine has never cut a
/// release itself: a build from `main` between the v2 and v3 commits can emit a v2
/// journal, and that journal must stay readable while a v3 line is refused by the
/// older binary.) The reduce path stays backward-tolerant of v1/v2 logs (which lack
/// this event); `TagState::github_release_delegated` is `#[serde(default)]`.
pub const JOURNAL_SCHEMA_VERSION: u32 = 3;

/// The five coordinator phases, in barrier order (ADR-0002): the derived
/// `PartialOrd`/`Ord` follows declaration order, so `DryRun < Build < Publish <
/// Tag < Dist` — the order the projection sorts phase records in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Dry-run-all: every adapter proves it *can* publish, nothing lands.
    DryRun,
    /// Build-all: every adapter produces its release artifact.
    Build,
    /// Publish-all: artifacts are pushed to their registries (point of no return
    /// per target — receipts are written per target before the next is tried).
    Publish,
    /// Tag-once: the coordinator (never an adapter) creates and pushes the tag
    /// and the GitHub Release.
    Tag,
    /// Dist (post-tag finalize): distribution targets whose artifact only exists
    /// *after* the tag is pushed are finalized here — the Homebrew formula, whose
    /// `url` is the just-created tag archive, is fetched, hashed, and its `.rb`
    /// written with the real `sha256`. Runs after [`Self::Tag`]; its `Ok`
    /// completion is what flips the run to [`RunStatus::Completed`]. A cut with no
    /// post-tag target still runs this barrier as a clean no-op so completion is
    /// uniform (ADR-0002 §2, extended by `release-engine-cut-cargo-dist-flow`).
    Dist,
}

impl Phase {
    /// The wire string for this phase (matches the `Serialize` derive), so text
    /// diagnostics and JSON never drift.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DryRun => "dry_run",
            Self::Build => "build",
            Self::Publish => "publish",
            Self::Tag => "tag",
            Self::Dist => "dist",
        }
    }

    /// Whether this phase is [`Self::Publish`] or a later barrier — i.e. a phase in
    /// which a target could have had a publish side-effect on a registry. Written as
    /// an explicit match (not `self >= Self::Publish`) so that inserting or
    /// reordering a phase forces a deliberate review of this safety predicate rather
    /// than silently changing it through the derived `Ord`. Used by the resume
    /// reconcile's "publish phase reached" signal (ADR-0003 §4).
    #[must_use]
    pub fn is_publish_or_later(self) -> bool {
        match self {
            Self::DryRun | Self::Build => false,
            Self::Publish | Self::Tag | Self::Dist => true,
        }
    }
}

/// How a phase barrier finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseOutcome {
    /// Every target cleared the barrier.
    Ok,
    /// The barrier failed (at least one target did not clear it); the run does
    /// not advance past a failed barrier.
    Failed,
}

/// Terminal-or-not status of a run, derived from the event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The run is live (created, not yet completed or abandoned).
    InProgress,
    /// The final [`Phase::Dist`] barrier completed [`PhaseOutcome::Ok`].
    Completed,
    /// A `run_abandoned` event was recorded (see [`RunState::abandon_reason`]).
    Abandoned,
}

impl RunStatus {
    /// The wire string for this status (matches the `Serialize` derive), so text
    /// diagnostics and JSON never drift.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }
}

/// The per-target publish receipt the journal persists — the fact "this exact
/// artifact landed" that resume/reconcile checks against the registry (the
/// remote is ground truth, ADR-0003 §4).
///
/// Written **per target before the next target is attempted**, never batched, so
/// an interrupted publish-all leaves an accurate record of exactly what landed.
/// Every descriptive field is optional so an adapter that cannot supply (say) a
/// content digest still records a usable receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishReceipt {
    /// The ecosystem the artifact was published to (`cargo`, `npm`, …).
    pub ecosystem: String,
    /// The published package/crate name, when the ecosystem has one.
    pub package: Option<String>,
    /// The version string that was published.
    pub version: String,
    /// The registry URL of the published artifact, when the adapter reports one.
    pub registry_url: Option<String>,
    /// A content digest of the published artifact, when available — the strongest
    /// signal for `verify()`'s `Matches`/`Conflicts` decision on resume.
    pub digest: Option<String>,
}

/// The progress of one release tag through its landing steps. Every field is a
/// monotonic (`false → true`) fact set by its own journal event, so re-applying a
/// tag event is a no-op. `created_local` and `pushed_remote` are orthogonal landing
/// facts; `github_release` vs `github_release_delegated` are the two
/// **mutually-exclusive** dispositions of the Release step (created-by-engine vs
/// delegated-to-CI) — the coordinator writes exactly one, and refuses to record a
/// second contradictory one (`crate::release::coordinator`'s tag phase), so the
/// illegal both-true state is unreachable for a valid run. They stay flat flags
/// (with a `clippy::struct_excessive_bools` allow) rather than a
/// `ReleaseDisposition` enum for consistency with the surrounding flat-flag style
/// and a `#[serde(default)]`-friendly additive wire shape; folding them into an
/// enum is a tracked cleanup, not a correctness fix given the write-time guard.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagState {
    /// The annotated tag exists in the local repository.
    pub created_local: bool,
    /// The tag has been pushed to the remote.
    pub pushed_remote: bool,
    /// The GitHub Release for the tag has been created.
    pub github_release: bool,
    /// The GitHub Release URL, once created.
    pub github_release_url: Option<String>,
    /// The GitHub Release was **delegated to CI** rather than created by the
    /// coordinator: the plan carried a CI-delegated target (e.g. `cargo-dist`'s
    /// `release.yml`) whose tag-triggered workflow owns Release creation and the
    /// cross-platform binary upload. Mutually exclusive in practice with
    /// [`Self::github_release`] — the coordinator either creates the Release or
    /// delegates it, never both — and, like the others, monotonic (`false → true`).
    /// Resume/verify treat a delegated Release as an intentional non-step, never a
    /// missing one to re-attempt (`coordinator-release-vs-cargo-dist-ownership`).
    /// `#[serde(default)]` so a pre-field manifest still deserializes (the manifest
    /// is disposable and rebuilt from the log anyway).
    #[serde(default)]
    pub github_release_delegated: bool,
}

/// One completed-phase record in the [`RunState`] projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseRecord {
    /// Which barrier completed.
    pub phase: Phase,
    /// How it completed.
    pub outcome: PhaseOutcome,
}

/// The payload of a journal event — the ADR-0002 event classes. Serialized
/// **internally tagged** on a `kind` discriminator, flattened into the
/// [`JournalEvent`] envelope so each JSONL line is one flat object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// The run was created: fixes its identity (`run_id`), the approved plan it
    /// executes (`plan_id`), and the ordered target set. Always the first event.
    RunCreated {
        /// The run's unique id (a ULID from the injected `IdGen`).
        run_id: String,
        /// The sealed, content-addressed plan id this run executes (ADR-0002).
        plan_id: String,
        /// The chosen release version this run publishes — the human's approved
        /// bump, sealed into `plan_id` and journalled as an input so `resume`
        /// (wave-3) can reconstruct the plan from the durable record alone
        /// (ADR-0002 §3; the plan module persists exactly `plan_id` + `version`).
        ///
        /// A **required** field of the v1 event, deliberately *not* `#[serde(default)]`:
        /// a `RunCreated` without it is corrupt (a resume must never fabricate an
        /// empty version and hash it into a wrong `plan_id`), so [`crate::release::journal::read_events`]
        /// refuses such a line with an actionable error rather than defaulting to `""`.
        version: String,
        /// The ordered target set (e.g. `["rust", "node"]`).
        targets: Vec<String>,
    },
    /// A phase barrier was entered.
    PhaseEntered {
        /// The barrier now in progress.
        phase: Phase,
    },
    /// A phase barrier completed.
    PhaseCompleted {
        /// The barrier that completed.
        phase: Phase,
        /// Its outcome.
        outcome: PhaseOutcome,
    },
    /// A target cleared its dry-run.
    TargetDryRun {
        /// The target id.
        target: String,
    },
    /// A target's release artifact was built.
    TargetBuilt {
        /// The target id.
        target: String,
    },
    /// A target was published — the point-of-no-return fact, with its receipt.
    TargetPublished {
        /// The target id.
        target: String,
        /// The receipt proving exactly what landed.
        receipt: PublishReceipt,
    },
    /// A target was cancelled (skipped) with a reason.
    TargetCancelled {
        /// The target id.
        target: String,
        /// Why it was cancelled.
        reason: String,
    },
    /// A **CI-delegated** target was skipped in the publish phase: its artifact is
    /// produced out-of-band by the tag-triggered CI (e.g. `cargo-dist`'s
    /// `release.yml`, a `release-please` merge job, or `PyPI`'s trusted-publisher
    /// workflow), not by the engine's `publish` step. Distinct from
    /// [`Self::TargetCancelled`] (a deliberate operator skip): a delegated target
    /// is *expected* to land via CI, so resume/verify treat it as neither
    /// engine-published nor missing/failed — the engine simply does not own it
    /// (`release-engine-cut-cargo-dist-flow`).
    TargetDelegated {
        /// The target id.
        target: String,
        /// The adapter identity that declared itself CI-delegated (its wire
        /// string, e.g. `"cargo-dist"`), for the operator-facing record.
        adapter: String,
    },
    /// The release tag was created locally.
    TagCreatedLocal {
        /// The tag name.
        tag: String,
    },
    /// The release tag was pushed to the remote.
    TagPushedRemote {
        /// The tag name.
        tag: String,
    },
    /// The GitHub Release for the tag was created.
    GithubReleaseCreated {
        /// The tag name.
        tag: String,
        /// The Release URL, when GitHub reports one.
        url: Option<String>,
    },
    /// The GitHub Release for the tag was **delegated to CI** — recorded in place
    /// of [`Self::GithubReleaseCreated`]. The plan carries a CI-delegated target
    /// (e.g. `cargo-dist`'s tag-triggered `release.yml`) whose workflow creates and
    /// finalizes the Release and uploads the cross-platform binaries, so the
    /// coordinator still creates and pushes the tag (that tag is what triggers CI)
    /// but deliberately does **not** create the Release itself — doing so would
    /// clash with CI over ownership of the same Release (creating it first, then CI
    /// either fails on "release already exists" or uploads into an engine-created
    /// stub). This fact is what lets resume/verify treat the missing engine-created
    /// Release as intentional rather than a step to re-attempt
    /// (`coordinator-release-vs-cargo-dist-ownership`).
    GithubReleaseDelegated {
        /// The tag name whose Release creation was delegated to CI.
        tag: String,
        /// The adapter identity whose CI owns the Release (its wire string, e.g.
        /// `"cargo-dist"`) — the operator-facing record of *what* the Release was
        /// delegated to, mirroring [`Self::TargetDelegated`]'s `adapter`.
        delegated_to: String,
    },
    /// The run was abandoned. Terminal; there is **no** auto-rollback (ADR-0002).
    RunAbandoned {
        /// Why the run was abandoned.
        reason: String,
    },
}

/// One line of `journal.jsonl`: a schema-versioned, sequenced, timestamped
/// envelope around an [`EventKind`].
///
/// The `kind` payload is `#[serde(flatten)]`ed so the on-disk line is a single
/// flat JSON object (`{"schema_version":1,"seq":3,"ts":…,"idempotency_key":…,
/// "kind":"target_published","target":"cargo","receipt":{…}}`), keeping it
/// `jq`/`tail`-friendly (AGENTS-AI-FIRST-CLI §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEvent {
    /// The durable schema version of this event (see [`JOURNAL_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Monotonic sequence number within the run, starting at 1. The reducer's
    /// high-water mark: an event at or below the applied `seq` is a no-op on
    /// replay (append-then-apply crash recovery, ADR-0003 §2).
    pub seq: u64,
    /// Event time as whole seconds since the Unix epoch, from the injected
    /// `Clock` (never wall-clock directly, so runs are deterministic under test).
    pub ts: u64,
    /// Stable semantic key identifying *what subject* this event records (e.g.
    /// `published:cargo`, `phase_completed:build`). It is **metadata**, not the
    /// append gate: it deliberately ignores the payload (a phase's `outcome`, a
    /// receipt's version), so it is safe for diagnostics and for a coordinator's
    /// own "have I already acted on this subject?" lookups, but it must **not**
    /// be used to suppress an append — a phase that completed `Failed` and then,
    /// after a resume, completes `Ok` shares this key yet is a distinct fact that
    /// must be recorded. Replay idempotency is provided by [`Self::seq`] (the
    /// high-water mark), not by this key.
    pub idempotency_key: String,
    /// The event payload, flattened into this envelope.
    #[serde(flatten)]
    pub kind: EventKind,
}

impl EventKind {
    /// The stable idempotency key for this event — its natural semantic identity,
    /// so a retried step (re-publishing an already-published target, re-entering a
    /// phase) resolves to the same key.
    ///
    /// This is **metadata only**: it is deliberately *not* used to suppress an
    /// append (see [`JournalEvent::idempotency_key`]). Two events can share a key
    /// yet be distinct facts that must both be recorded — a `phase_completed`
    /// `Failed` and, after a resume, a `phase_completed` `Ok` for the same phase.
    /// Replay idempotency comes from [`JournalEvent::seq`] (the watermark), never
    /// from this key.
    #[must_use]
    pub fn idempotency_key(&self) -> String {
        match self {
            Self::RunCreated { .. } => "run_created".to_string(),
            Self::PhaseEntered { phase } => format!("phase_entered:{}", phase.as_str()),
            Self::PhaseCompleted { phase, .. } => {
                format!("phase_completed:{}", phase.as_str())
            }
            Self::TargetDryRun { target } => format!("dry_run:{target}"),
            Self::TargetBuilt { target } => format!("built:{target}"),
            Self::TargetPublished { target, .. } => format!("published:{target}"),
            Self::TargetCancelled { target, .. } => format!("cancelled:{target}"),
            Self::TargetDelegated { target, .. } => format!("delegated:{target}"),
            Self::TagCreatedLocal { tag } => format!("tag_created_local:{tag}"),
            Self::TagPushedRemote { tag } => format!("tag_pushed_remote:{tag}"),
            Self::GithubReleaseCreated { tag, .. } => format!("github_release_created:{tag}"),
            Self::GithubReleaseDelegated { tag, .. } => format!("github_release_delegated:{tag}"),
            Self::RunAbandoned { .. } => "run_abandoned".to_string(),
        }
    }
}

/// The materialized run state — the projection reduced from the event log and
/// cached in `manifest.json`.
///
/// Every collection is a `BTree*`/sorted `Vec`, so the serialized manifest is
/// **byte-deterministic** for a given event stream: the same events always
/// produce the same JSON, which is what makes the manifest a trustworthy cache
/// of the log. It is disposable — rebuild it any time with
/// [`crate::release::journal::reduce`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunState {
    /// Durable schema version of this state document.
    pub schema_version: u32,
    /// The run's unique id (empty only before the `RunCreated` event).
    pub run_id: String,
    /// The sealed plan id this run executes.
    pub plan_id: String,
    /// The chosen release version this run publishes (from the `RunCreated`
    /// event) — the input `resume` reconstructs the plan against. Populated from
    /// the required event field; the manifest is disposable and always rebuilt
    /// from the log, so no cross-version default is needed here.
    pub version: String,
    /// The ordered target set, as declared at creation.
    pub targets: Vec<String>,
    /// The high-water sequence number folded into this state — the append-then-
    /// apply watermark (ADR-0003 §2).
    pub applied_seq: u64,
    /// Derived run status.
    pub status: RunStatus,
    /// The phase currently in progress, if any.
    pub current_phase: Option<Phase>,
    /// Completed phases with their outcomes, sorted by phase order.
    pub phases: Vec<PhaseRecord>,
    /// Targets that cleared dry-run.
    pub dry_run: BTreeSet<String>,
    /// Targets whose artifact was built.
    pub built: BTreeSet<String>,
    /// Published targets → their receipts.
    pub published: BTreeMap<String, PublishReceipt>,
    /// Cancelled targets → their reasons.
    pub cancelled: BTreeMap<String, String>,
    /// CI-delegated targets (their ids): skipped in publish because a
    /// tag-triggered CI job produces their artifact, not the engine. Neither
    /// engine-published nor missing/failed (`release-engine-cut-cargo-dist-flow`).
    /// `#[serde(default)]` so a v1 manifest that predates the field still
    /// deserializes (the manifest is disposable and rebuilt from the log anyway).
    #[serde(default)]
    pub delegated: BTreeSet<String>,
    /// Release tags → their landing progress.
    pub tags: BTreeMap<String, TagState>,
    /// The reason recorded by a `run_abandoned` event, if any.
    pub abandon_reason: Option<String>,
    /// Timestamp of the `RunCreated` event.
    pub created_ts: u64,
    /// Timestamp of the most recently applied event.
    pub updated_ts: u64,
}

impl RunState {
    /// The empty pre-`RunCreated` state the reducer folds events into. Carries
    /// the current [`JOURNAL_SCHEMA_VERSION`] and [`RunStatus::InProgress`];
    /// identity fields are filled by the first (`RunCreated`) event.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            run_id: String::new(),
            plan_id: String::new(),
            version: String::new(),
            targets: Vec::new(),
            applied_seq: 0,
            status: RunStatus::InProgress,
            current_phase: None,
            phases: Vec::new(),
            dry_run: BTreeSet::new(),
            built: BTreeSet::new(),
            published: BTreeMap::new(),
            cancelled: BTreeMap::new(),
            delegated: BTreeSet::new(),
            tags: BTreeMap::new(),
            abandon_reason: None,
            created_ts: 0,
            updated_ts: 0,
        }
    }
}

impl Default for RunState {
    fn default() -> Self {
        Self::empty()
    }
}
