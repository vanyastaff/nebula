//! Job-dispatch message DTO and routing types.
//!
//! `JobDispatchMsg` is the durable unit of work enqueued by the emitter and
//! pulled by the orchestrator.  The routing predicate is
//! exact flavor equality AND `required_plugins ⊆ available_plugins`: a worker may claim a job only
//! when its advertised set is a superset of the job's `required_plugins`.
//! `required_plugin_key` is kept as an index-friendly pre-filter (sound
//! because the DTO invariant guarantees `required_plugins ⊇ {required_plugin_key}`).
use nebula_core::PluginKey;
use nebula_core::WorkerFlavorRevisionId;

use crate::Scope;
use crate::dto::ControlCommand;

/// One queued job-dispatch message.
///
/// `id` is a typed 16-byte ULID (raw bytes). `event_id` preserves the
/// source-natural idempotency key associated with the accepted start.
///
/// Construct via [`JobDispatchMsg::new`]; struct literal syntax is
/// unavailable from external crates (`#[non_exhaustive]`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
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
    /// Deduplication is owned by [`crate::store::StartAcceptanceStore`]; this
    /// field is retained as dispatch metadata and never grants acceptance.
    pub event_id: Option<String>,
    /// The primary required plugin (the trigger's plugin); an element of
    /// `required_plugins`; used as the index pre-filter.
    ///
    /// The orchestrator claims only rows whose `required_plugin_key` is a
    /// member of the worker's `available_plugins`.
    pub required_plugin_key: PluginKey,
    /// Full set of plugin keys required by this job (trigger + enabled nodes,
    /// deduplicated and sorted).  Superset of `{required_plugin_key}`.
    /// Stored as a JSON array of strings in the backend.
    pub required_plugins: Vec<PluginKey>,
    /// Optional W3C `traceparent` captured at enqueue time.
    pub w3c_traceparent: Option<String>,
    /// Times this row was reclaimed back to `Pending` after a crashed runner.
    pub reclaim_count: u32,
    /// Exact worker-flavor revision required to claim this job.
    ///
    /// A worker whose exact flavor does not match cannot claim the row.
    pub required_worker_flavor_id: WorkerFlavorRevisionId,
}

impl JobDispatchMsg {
    /// Construct a job-dispatch message.
    ///
    /// **Invariant:** `required_plugins` must contain `required_plugin_key`.
    /// The storage backends rely on this to use `required_plugin_key` as a
    /// sound index pre-filter for the superset routing predicate
    /// (`required_plugins ⊆ available_plugins`).  The
    /// `DefinitionRoutingResolver` always inserts the plugin key into
    /// `required_plugins`, so real producers satisfy this invariant by
    /// construction; a `debug_assert` catches violations in test.
    // guard-justified: constructor over all DTO fields; a builder adds no safety
    // for an internal #[non_exhaustive] record whose fields are all independent.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        id: [u8; 16],
        execution_id: impl Into<String>,
        command: ControlCommand,
        scope: Scope,
        payload: serde_json::Value,
        event_id: Option<impl Into<String>>,
        required_plugin_key: PluginKey,
        required_plugins: Vec<PluginKey>,
        w3c_traceparent: Option<impl Into<String>>,
        reclaim_count: u32,
        required_worker_flavor_id: WorkerFlavorRevisionId,
    ) -> Self {
        debug_assert!(
            required_plugins.contains(&required_plugin_key),
            "required_plugins must contain required_plugin_key \
             (invariant required by the superset routing pre-filter): \
             required_plugin_key = {required_plugin_key:?}, \
             required_plugins = {required_plugins:?}"
        );
        Self {
            id,
            execution_id: execution_id.into(),
            command,
            scope,
            payload,
            event_id: event_id.map(Into::into),
            required_plugin_key,
            required_plugins,
            w3c_traceparent: w3c_traceparent.map(Into::into),
            reclaim_count,
            required_worker_flavor_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_dispatch_requires_exact_well_formed_flavor_identity() {
        let message = JobDispatchMsg::new(
            [1; 16],
            "execution",
            ControlCommand::Start,
            Scope::new("workspace", "org"),
            serde_json::Value::Null,
            None::<String>,
            "plugin".parse().unwrap(),
            vec!["plugin".parse().unwrap()],
            None::<String>,
            0,
            WorkerFlavorRevisionId::from_bytes([0x22; 32]),
        );
        let encoded = serde_json::to_value(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<JobDispatchMsg>(&serde_json::to_string(&encoded).unwrap())
                .unwrap(),
            message
        );
        for invalid in [
            serde_json::Value::Null,
            serde_json::json!("ab"),
            serde_json::json!("GG".repeat(32)),
        ] {
            let mut altered = encoded.clone();
            altered["required_worker_flavor_id"] = invalid;
            assert!(
                serde_json::from_str::<JobDispatchMsg>(&serde_json::to_string(&altered).unwrap())
                    .is_err()
            );
        }
        let mut missing = encoded;
        missing
            .as_object_mut()
            .unwrap()
            .remove("required_worker_flavor_id");
        assert!(
            serde_json::from_str::<JobDispatchMsg>(&serde_json::to_string(&missing).unwrap())
                .is_err()
        );
    }
}
