//! Execution repository port.
//!
//! Defines the persistence interface for execution state, journals, and leases.
//! Backend drivers implement this trait to provide durable execution tracking.

use std::time::Duration;

use async_trait::async_trait;
use nebula_core::ExecutionId;

use crate::error::PortsError;

/// Persistence and coordination interface for workflow executions.
///
/// Covers three concerns:
/// - **State**: versioned execution state with compare-and-swap transitions
/// - **Journal**: append-only event log for replay and audit
/// - **Leases**: distributed locking so only one worker drives an execution
#[async_trait]
pub trait ExecutionRepo: Send + Sync {
    /// Get execution state. Returns `(version, state_json)` or `None`.
    async fn get_state(
        &self,
        id: ExecutionId,
    ) -> Result<Option<(u64, serde_json::Value)>, PortsError>;

    /// Compare-and-swap state transition. Returns `true` if successful.
    async fn transition(
        &self,
        id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, PortsError>;

    /// Get the full journal (event log) for an execution.
    async fn get_journal(&self, id: ExecutionId) -> Result<Vec<serde_json::Value>, PortsError>;

    /// Append a journal entry.
    async fn append_journal(
        &self,
        id: ExecutionId,
        entry: serde_json::Value,
    ) -> Result<(), PortsError>;

    /// Acquire an exclusive execution lease.
    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        ttl: Duration,
    ) -> Result<bool, PortsError>;

    /// Renew an existing lease.
    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool, PortsError>;

    /// Release a lease.
    async fn release_lease(&self, id: ExecutionId, holder: &str) -> Result<bool, PortsError>;
}
