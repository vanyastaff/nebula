use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;

/// Transaction vote from a participant in a distributed transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionVote {
    /// Ready to commit — all preconditions met.
    Commit,
    /// Must abort — preconditions failed.
    Abort,
}

/// Outcome decided by the saga coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionOutcome {
    /// All participants voted Commit — finalize the transaction.
    Committed,
    /// At least one participant voted Abort — run compensation.
    RolledBack,
}

/// Result of the prepare phase.
#[derive(Debug, Clone)]
pub struct PrepareResult<T> {
    /// Unique transaction identifier.
    pub transaction_id: String,
    /// Data needed to undo this step if the transaction aborts.
    pub rollback_data: T,
    /// This participant's vote.
    pub vote: TransactionVote,
    /// Deadline after which the coordinator should assume abort.
    pub expires_at: DateTime<Utc>,
}

/// Action participating in a distributed transaction (saga pattern).
///
/// Implements two-phase execution:
/// 1. **Prepare**: validate preconditions, reserve resources, return a vote.
/// 2. **Commit** or **Compensate**: finalize or undo based on coordinator decision.
///
/// The engine acts as the saga coordinator — it collects votes from all
/// participants and decides the outcome. If any participant votes `Abort`,
/// all participants are asked to compensate.
///
/// # Type Parameters
///
/// - `Input`: data received for this transaction step.
/// - `Output`: data produced on successful commit.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::*;
/// use nebula_action::transactional::*;
/// use async_trait::async_trait;
///
/// struct DebitAccount {
///     meta: ActionMetadata,
/// }
///
/// #[async_trait]
/// impl TransactionalAction for DebitAccount {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///
///     async fn prepare(
///         &self, input: Self::Input, ctx: &ActionContext,
///     ) -> Result<PrepareResult<serde_json::Value>, ActionError> {
///         ctx.check_cancelled()?;
///         // Reserve funds, return rollback data
///         Ok(PrepareResult {
///             transaction_id: "tx-123".into(),
///             rollback_data: serde_json::json!({"refund": 100}),
///             vote: TransactionVote::Commit,
///             expires_at: chrono::Utc::now() + chrono::Duration::seconds(30),
///         })
///     }
///
///     async fn commit(
///         &self, _transaction_id: &str, ctx: &ActionContext,
///     ) -> Result<Self::Output, ActionError> {
///         ctx.check_cancelled()?;
///         Ok(serde_json::json!({"status": "debited"}))
///     }
///
///     async fn compensate(
///         &self, _transaction_id: &str, rollback_data: serde_json::Value, ctx: &ActionContext,
///     ) -> Result<(), ActionError> {
///         ctx.check_cancelled()?;
///         // Refund using rollback_data
///         let _ = rollback_data;
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait TransactionalAction: Action {
    /// Input data for this transaction step.
    type Input: Send + Sync + 'static;
    /// Output data produced on successful commit.
    type Output: Send + Sync + 'static;

    /// Phase 1: Prepare — validate, reserve resources, cast a vote.
    ///
    /// Must be idempotent (safe to retry). On success, returns a
    /// `PrepareResult` with rollback data and a vote.
    async fn prepare(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<PrepareResult<serde_json::Value>, ActionError>;

    /// Phase 2a: Commit — finalize the transaction (all participants voted Commit).
    ///
    /// Called only when the coordinator decides to commit.
    /// Must be idempotent.
    async fn commit(
        &self,
        transaction_id: &str,
        ctx: &ActionContext,
    ) -> Result<Self::Output, ActionError>;

    /// Phase 2b: Compensate — undo the work (at least one participant voted Abort).
    ///
    /// Called only when the coordinator decides to roll back.
    /// Receives the `rollback_data` from the prepare phase.
    /// Must be idempotent — may be retried on failure.
    async fn compensate(
        &self,
        transaction_id: &str,
        rollback_data: serde_json::Value,
        ctx: &ActionContext,
    ) -> Result<(), ActionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_vote_equality() {
        assert_eq!(TransactionVote::Commit, TransactionVote::Commit);
        assert_ne!(TransactionVote::Commit, TransactionVote::Abort);
    }

    #[test]
    fn transaction_outcome_equality() {
        assert_eq!(TransactionOutcome::Committed, TransactionOutcome::Committed);
        assert_ne!(
            TransactionOutcome::Committed,
            TransactionOutcome::RolledBack
        );
    }

    #[test]
    fn prepare_result_construction() {
        let result = PrepareResult {
            transaction_id: "tx-abc".into(),
            rollback_data: serde_json::json!({"undo": true}),
            vote: TransactionVote::Commit,
            expires_at: Utc::now(),
        };
        assert_eq!(result.transaction_id, "tx-abc");
        assert_eq!(result.vote, TransactionVote::Commit);
    }

    #[test]
    fn vote_serialization() {
        let json = serde_json::to_string(&TransactionVote::Abort).unwrap();
        assert!(json.contains("Abort"));
        let deserialized: TransactionVote = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, TransactionVote::Abort);
    }

    #[test]
    fn outcome_serialization() {
        let json = serde_json::to_string(&TransactionOutcome::RolledBack).unwrap();
        let deserialized: TransactionOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, TransactionOutcome::RolledBack);
    }
}
