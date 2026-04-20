//! Trigger repository — definitions, events, cron slots.

use std::future::Future;

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
pub trait TriggerRepo: Send + Sync {
    // ── Trigger definitions ─────────────────────────────────────────────

    /// Insert a new trigger.
    fn create(&self, trigger: &TriggerRow)
    -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a trigger by ID.
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<TriggerRow>, StorageError>> + Send;

    /// Update a trigger with CAS on `version`.
    fn update(
        &self,
        trigger: &TriggerRow,
        expected_version: i64,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Soft-delete a trigger.
    fn soft_delete(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List all active triggers in a workspace (non-archived, non-deleted).
    fn list_active(
        &self,
        workspace_id: &[u8],
    ) -> impl Future<Output = Result<Vec<TriggerRow>, StorageError>> + Send;

    /// List all active cron triggers across workspaces (for scheduler tick).
    fn list_active_cron(
        &self,
    ) -> impl Future<Output = Result<Vec<TriggerRow>, StorageError>> + Send;

    // ── Event inbox ─────────────────────────────────────────────────────

    /// Insert a trigger event. Returns `Ok(false)` if the event was a
    /// duplicate (dedup on `UNIQUE(trigger_id, event_id)`), `Ok(true)`
    /// when a new row was inserted.
    fn insert_event(
        &self,
        event: &TriggerEventRow,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Claim up to `batch_size` pending events for a dispatcher.
    fn claim_events(
        &self,
        claimed_by: &[u8],
        batch_size: u32,
    ) -> impl Future<Output = Result<Vec<TriggerEventRow>, StorageError>> + Send;

    /// Mark an event as dispatched; links it to the created execution.
    fn mark_dispatched(
        &self,
        event_id: &[u8],
        execution_id: &[u8],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Mark an event as failed.
    fn mark_event_failed(
        &self,
        event_id: &[u8],
        reason: &str,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    // ── Cron fire slots ─────────────────────────────────────────────────

    /// Attempt to claim a cron slot. Returns `true` when this caller
    /// claimed the slot, `false` when another dispatcher got it first
    /// (unique-constraint violation).
    fn claim_cron_slot(
        &self,
        slot: &CronFireSlotRow,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Link a claimed slot to the created execution.
    fn set_cron_slot_execution(
        &self,
        trigger_id: &[u8],
        scheduled_for: DateTime<Utc>,
        execution_id: &[u8],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Clean up cron slots older than `retention`. Returns count deleted.
    fn cleanup_cron_slots(
        &self,
        retention: std::time::Duration,
    ) -> impl Future<Output = Result<u64, StorageError>> + Send;
}
