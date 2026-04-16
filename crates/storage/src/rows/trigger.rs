//! Row types for the trigger layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Table: `triggers`
///
/// Defines how and when a workflow is started. Supports manual,
/// cron, webhook, event, and polling trigger kinds.
#[derive(Debug, Clone)]
pub struct TriggerRow {
    /// `trig_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub workspace_id: Vec<u8>,
    pub workflow_id: Vec<u8>,
    pub slug: String,
    pub display_name: String,
    /// `'manual'` / `'cron'` / `'webhook'` / `'event'` / `'polling'`.
    pub kind: String,
    pub config: Value,
    /// `'active'` / `'paused'` / `'archived'`.
    pub state: String,
    /// `ServiceAccountId`; `None` uses workspace default.
    pub run_as: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
    pub created_by: Vec<u8>,
    /// Optimistic concurrency version.
    pub version: i64,
    pub deleted_at: Option<DateTime<Utc>>,
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
