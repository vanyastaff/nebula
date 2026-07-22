//! Row types for the trigger layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Table: `triggers`
///
/// Defines how and when a workflow is started. Supports manual,
/// cron, webhook, event, and polling trigger kinds.
#[derive(Clone)]
pub struct TriggerRow {
    /// `trig_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub workspace_id: Vec<u8>,
    pub workflow_id: Vec<u8>,
    pub slug: String,
    pub display_name: String,
    /// `'manual'` / `'cron'` / `'webhook'` / `'event'` / `'polling'`.
    pub kind: String,
    /// Kind-namespaced configuration JSONB.
    ///
    /// Each `kind` owns a top-level key inside the JSON object so
    /// fields cannot collide across kinds:
    ///
    /// - `kind = 'cron'` → `{ "schedule": "...", "timezone": "..." }`
    ///   at the top level (legacy shape preserved for existing rows).
    /// - `kind = 'webhook'` → `{ "webhook_activation": WebhookActivationSpec }`.
    /// - `kind = 'event'` → `{ "event_types": [...] }`.
    ///
    /// PG migration `0025_triggers_config_namespace_comment.sql`
    /// attaches the canonical contract as `COMMENT ON COLUMN
    /// triggers.config` so DBA tooling sees the same shape. See
    /// [`crate::rows::WebhookActivationSpec`] for the webhook decoder.
    pub config: Value,
    /// `'active'` / `'paused'` / `'archived'`.
    pub state: String,
    /// Operator-supplied URL slug for `kind = 'webhook'` triggers.
    /// `NULL` for every other kind. Indexed (migration 0018) for
    /// O(1) `(org, ws, slug)` → trigger lookup at request time.
    pub webhook_path: Option<String>,
    /// `ServiceAccountId`; `None` uses workspace default.
    pub run_as: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
    pub created_by: Vec<u8>,
    /// Optimistic concurrency version.
    pub version: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for TriggerRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TriggerRow")
            .field("id", &self.id)
            .field("workspace_id", &self.workspace_id)
            .field("workflow_id", &self.workflow_id)
            .field("slug", &self.slug)
            .field("display_name", &self.display_name)
            .field("kind", &self.kind)
            .field("config", &"[redacted]")
            .field("state", &self.state)
            .field("webhook_path", &self.webhook_path)
            .field("run_as", &self.run_as)
            .field("created_at", &self.created_at)
            .field("created_by", &self.created_by)
            .field("version", &self.version)
            .field("deleted_at", &self.deleted_at)
            .finish()
    }
}

#[cfg(test)]
mod trigger_row_debug_tests {
    use chrono::Utc;

    use super::TriggerRow;

    #[test]
    fn trigger_row_debug_redacts_config() {
        const CANARY: &str = "TRIGGER_CONFIG_AUTHORITY_CANARY-c1e8";
        let row = TriggerRow {
            id: vec![1],
            workspace_id: vec![2],
            workflow_id: vec![3],
            slug: "hook".to_owned(),
            display_name: "Hook".to_owned(),
            kind: "webhook".to_owned(),
            config: serde_json::json!({ "challenge_token": CANARY }),
            state: "active".to_owned(),
            webhook_path: Some("hook".to_owned()),
            run_as: None,
            created_at: Utc::now(),
            created_by: vec![4],
            version: 1,
            deleted_at: None,
        };

        let debug = format!("{row:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("[redacted]"));
    }
}

/// Table: `trigger_events`
///
/// Inbound trigger event inbox with dedup enforcement via
/// `UNIQUE (trigger_id, event_id)`.
#[derive(Debug, Clone)]
pub struct TriggerEventRow {
    /// `evt_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub trigger_id: Vec<u8>,
    /// Author-configured or fallback hash for dedup.
    pub event_id: String,
    pub received_at: DateTime<Utc>,
    /// `'pending'` / `'claimed'` / `'dispatched'` / `'failed'`.
    pub claim_state: String,
    /// Dispatcher `node_id`.
    pub claimed_by: Option<Vec<u8>>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub payload: Value,
    /// Set after the execution is created.
    pub execution_id: Option<Vec<u8>>,
    pub metadata: Option<Value>,
}

/// Table: `cron_fire_slots`
///
/// Leaderless cron coordination. Multiple dispatchers race to
/// claim each slot; the unique constraint prevents double-fire.
/// Primary key: `(trigger_id, scheduled_for)`.
#[derive(Debug, Clone)]
pub struct CronFireSlotRow {
    pub trigger_id: Vec<u8>,
    pub scheduled_for: DateTime<Utc>,
    /// Dispatcher `node_id` that claimed this slot.
    pub claimed_by: Vec<u8>,
    pub claimed_at: DateTime<Utc>,
    /// Populated after the execution is created.
    pub execution_id: Option<Vec<u8>>,
}

/// Table: `pending_signals`
///
/// Signals received for suspended execution nodes. Consumed
/// when the node resumes.
#[derive(Debug, Clone)]
pub struct PendingSignalRow {
    /// 16-byte BYTEA primary key.
    pub id: Vec<u8>,
    /// FK to `execution_nodes.id`.
    pub node_attempt_id: Vec<u8>,
    pub signal_name: String,
    pub payload: Option<Value>,
    pub received_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}
