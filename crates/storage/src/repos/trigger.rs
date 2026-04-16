//! Trigger repository — definitions, events, cron slots.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{
    error::StorageError,
    rows::{CronFireSlotRow, TriggerEventRow, TriggerRow},
};

/// Trigger, event, and cron-slot storage.
///
/// Spec 16 layer 5. Event inbox uses `UNIQUE (trigger_id, event_id)`
/// for dedup; cron slots use `PRIMARY KEY (trigger_id, scheduled_for)`
/// for leaderless firing coordination.
#[async_trait]
pub trait TriggerRepo: Send + Sync {
    // ── Trigger definitions ─────────────────────────────────────────────

    /// Insert a new trigger.
    async fn create(&self, trigger: &TriggerRow) -> Result<(), StorageError>;

    /// Fetch a trigger by ID.
    async fn get(&self, id: &[u8]) -> Result<Option<TriggerRow>, StorageError>;

    /// Update a trigger with CAS on `version`.
    async fn update(&self, trigger: &TriggerRow, expected_version: i64)
    -> Result<(), StorageError>;

    /// Soft-delete a trigger.
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError>;

    /// List all active triggers in a workspace (non-archived, non-deleted).
    async fn list_active(&self, workspace_id: &[u8]) -> Result<Vec<TriggerRow>, StorageError>;

    /// List all active cron triggers across workspaces (for scheduler tick).
    async fn list_active_cron(&self) -> Result<Vec<TriggerRow>, StorageError>;

    // ── Event inbox ─────────────────────────────────────────────────────

    /// Insert a trigger event. Returns `Ok(false)` if the event was a
    /// duplicate (dedup on `UNIQUE(trigger_id, event_id)`), `Ok(true)`
    /// when a new row was inserted.
    async fn insert_event(&self, event: &TriggerEventRow) -> Result<bool, StorageError>;

    /// Claim up to `batch_size` pending events for a dispatcher.
    async fn claim_events(
        &self,
        claimed_by: &[u8],
        batch_size: u32,
    ) -> Result<Vec<TriggerEventRow>, StorageError>;

    /// Mark an event as dispatched; links it to the created execution.
    async fn mark_dispatched(
        &self,
        event_id: &[u8],
        execution_id: &[u8],
    ) -> Result<(), StorageError>;

    /// Mark an event as failed.
    async fn mark_event_failed(&self, event_id: &[u8], reason: &str) -> Result<(), StorageError>;

    // ── Cron fire slots ─────────────────────────────────────────────────

    /// Attempt to claim a cron slot. Returns `true` when this caller
    /// claimed the slot, `false` when another dispatcher got it first
    /// (unique-constraint violation).
    async fn claim_cron_slot(&self, slot: &CronFireSlotRow) -> Result<bool, StorageError>;

    /// Link a claimed slot to the created execution.
    async fn set_cron_slot_execution(
        &self,
        trigger_id: &[u8],
        scheduled_for: DateTime<Utc>,
        execution_id: &[u8],
    ) -> Result<(), StorageError>;

    /// Clean up cron slots older than `retention`. Returns count deleted.
    async fn cleanup_cron_slots(&self, retention: std::time::Duration)
    -> Result<u64, StorageError>;
}
