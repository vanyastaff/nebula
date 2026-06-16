//! Job-dispatch message DTO and routing types.
//!
//! `JobDispatchMsg` is the durable unit of work enqueued by the emitter and
//! pulled by the orchestrator.  The routing key is `required_plugin_key`
//! matched against a worker's `capability_tags`; `target_flavor_sha` is a
//! separate version-pin guard and is never used for routing.
use crate::Scope;
use crate::dto::ControlCommand;
use serde::{Deserialize, Serialize};

/// Opaque capability routing tag (advertised PluginKey strings).
///
/// A worker advertises the set of `CapabilityTag`s it supports; the
/// orchestrator claims only rows whose `required_plugin_key` is a member of
/// that set.  The tag is the canonical `PluginKey` string form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityTag(pub String);

impl CapabilityTag {
    /// Borrow the inner tag string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for CapabilityTag {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for CapabilityTag {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Whether the compose wrote new rows or found an existing dedup guard.
///
/// Returned by [`crate::store::TriggerDedupInbox::claim_and_materialize_start`].
/// The `execution_id` is the EFFECTIVE id — the first winner's id on both
/// `Dispatched` and `Duplicate` outcomes, so callers always get the canonical
/// id for the event without a separate read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "callers must read the effective execution id and the dispatch kind"]
#[non_exhaustive]
pub struct DispatchOutcome {
    /// The effective execution id: the new row's id on `Dispatched`; the
    /// original winner's id (read back in-transaction) on `Duplicate`.
    pub execution_id: String,
    /// Whether this was the first write or a deduplicated repeat.
    pub kind: DispatchKind,
}

impl DispatchOutcome {
    /// Construct a dispatch outcome.
    pub fn new(execution_id: impl Into<String>, kind: DispatchKind) -> Self {
        Self {
            execution_id: execution_id.into(),
            kind,
        }
    }
}

/// Whether the compose performed new writes or found the event already handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DispatchKind {
    /// All three rows were written atomically (dedup guard + Start job +
    /// execution row).  First writer wins.
    Dispatched,
    /// A dedup guard for this `(scope, trigger_id, event_id)` was already
    /// present; no new rows were written.
    Duplicate,
}

/// One queued job-dispatch message.
///
/// `id` is a typed 16-byte ULID (raw bytes).  `event_id` is `None` when the
/// caller wants a single unconditional dispatch with no dedup row; it is
/// `Some` (a source-natural idempotency key) when the trigger-dedup inbox
/// must guard against duplicate fan-out.
///
/// Construct via [`JobDispatchMsg::new`]; struct literal syntax is
/// unavailable from external crates (`#[non_exhaustive]`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct JobDispatchMsg {
    /// 16-byte ULID primary key (raw bytes).
    pub id: [u8; 16],
    /// Target execution id (opaque string form).
    pub execution_id: String,
    /// Control command to deliver (typically `Start`).
    pub command: ControlCommand,
    /// Tenant scope this message belongs to.
    pub scope: Scope,
    /// Arbitrary payload forwarded to the worker unchanged.
    pub payload: serde_json::Value,
    /// Source-natural dedup key.
    ///
    /// `None` ⇒ dispatch once, no dedup row written.  `Some` ⇒ the
    /// trigger-dedup inbox guards against duplicate fan-out with the scoped
    /// constraint `UNIQUE(workspace_id, org_id, trigger_id, event_id)`.
    /// A fresh ULID is never the right value here — it would defeat the dedup
    /// invariant.
    pub event_id: Option<String>,
    /// Version-pin guard (SHA of the plugin flavor).  Not a routing key.
    pub target_flavor_sha: String,
    /// Routing key: the advertised `PluginKey` this job requires.
    ///
    /// The orchestrator claims only rows whose `required_plugin_key` is a
    /// member of a worker's advertised `capability_tags`.
    pub required_plugin_key: String,
    /// Full set of capability tags accepted by this job (superset of
    /// `required_plugin_key`).  Stored as a JSON array in the backend.
    pub capability_tags: Vec<CapabilityTag>,
    /// Optional W3C `traceparent` captured at enqueue time.
    pub w3c_traceparent: Option<String>,
    /// Times this row was reclaimed back to `Pending` after a crashed runner.
    pub reclaim_count: u32,
}

impl JobDispatchMsg {
    /// Construct a job-dispatch message.
    ///
    /// `capability_tags` must include `required_plugin_key`; callers are
    /// responsible for that invariant (no enforcement here to keep the
    /// constructor cheap).
    // guard-justified: constructor over all DTO fields; a builder adds no safety
    // for an internal #[non_exhaustive] record whose fields are all independent.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: [u8; 16],
        execution_id: impl Into<String>,
        command: ControlCommand,
        scope: Scope,
        payload: serde_json::Value,
        event_id: Option<impl Into<String>>,
        target_flavor_sha: impl Into<String>,
        required_plugin_key: impl Into<String>,
        capability_tags: Vec<CapabilityTag>,
        w3c_traceparent: Option<impl Into<String>>,
        reclaim_count: u32,
    ) -> Self {
        Self {
            id,
            execution_id: execution_id.into(),
            command,
            scope,
            payload,
            event_id: event_id.map(Into::into),
            target_flavor_sha: target_flavor_sha.into(),
            required_plugin_key: required_plugin_key.into(),
            capability_tags,
            w3c_traceparent: w3c_traceparent.map(Into::into),
            reclaim_count,
        }
    }
}
