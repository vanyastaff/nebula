//! Rotation Transaction
//!
//! Tracks the state and metadata of a credential rotation operation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::CredentialId;

use super::error::RotationResult;
use super::state::RotationState;

/// Unique identifier for a rotation transaction
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct RotationId(Uuid);

impl RotationId {
    /// Generate a new rotation ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the inner UUID
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl AsRef<Uuid> for RotationId {
    fn as_ref(&self) -> &Uuid {
        &self.0
    }
}

impl Default for RotationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RotationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for RotationId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

/// Rotation transaction tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationTransaction {
    /// Unique transaction identifier
    pub id: RotationId,

    /// Credential being rotated
    pub credential_id: CredentialId,

    /// Current state of the rotation
    pub state: RotationState,

    /// Version of the old credential
    pub old_version: u32,

    /// Version of the new credential (set when created)
    pub new_version: Option<u32>,

    /// When rotation started
    pub started_at: DateTime<Utc>,

    /// When rotation completed (Committed or RolledBack)
    pub completed_at: Option<DateTime<Utc>>,

    /// Validation result if validation was performed
    pub validation_result: Option<ValidationResult>,

    /// Backup ID for disaster recovery
    pub backup_id: Option<BackupId>,

    /// When grace period ends (set when Committed)
    pub grace_period_end: Option<DateTime<Utc>>,

    /// Error message if rotation failed
    pub error_message: Option<String>,

    /// Manual rotation metadata (if triggered manually)
    pub manual_rotation: Option<ManualRotation>,

    /// Rollback strategy for this transaction
    pub rollback_strategy: RollbackStrategy,

    /// Two-phase commit transaction phase (if using 2PC)
    pub transaction_phase: Option<TransactionPhase>,

    /// Optimistic lock for concurrency control (if using locking)
    pub optimistic_lock: Option<OptimisticLock>,
}

/// Validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether validation passed
    pub passed: bool,

    /// Validation message
    pub message: String,

    /// When validation was performed
    pub validated_at: DateTime<Utc>,
}

/// Unique identifier for a backup
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct BackupId(Uuid);

/// Optimistic lock for concurrent rotation prevention
///
/// Uses version numbers to detect concurrent modifications and prevent
/// conflicting rotation operations on the same credential.
///
/// # T091: Optimistic Lock
///
/// # Thread Safety
///
/// **IMPORTANT**: This type is NOT thread-safe for in-memory concurrent access.
/// It's designed to be persisted to a database where the storage provider
/// ensures atomicity through database transactions (e.g., SQL `UPDATE ... WHERE version = ?`).
///
/// For in-memory concurrent access across threads, wrap in `Arc<Mutex<OptimisticLock>>`
/// or use a proper distributed lock service.
///
/// # Example (Storage-backed)
///
/// ```rust,ignore
/// use nebula_credential::rotation::OptimisticLock;
///
/// // Read from database
/// let mut lock = storage.load_lock(&credential_id)?;
///
/// // Try to acquire lock
/// lock.acquire_lock("transaction-123")?;
///
/// // Perform rotation with CAS
/// lock.compare_and_swap(current_version, new_version)?;
///
/// // Write back to database atomically
/// storage.save_lock(&lock)?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimisticLock {
    /// Credential being locked
    pub credential_id: CredentialId,

    /// Expected version number
    pub expected_version: u32,

    /// New version number (set after successful operation)
    pub new_version: Option<u32>,

    /// When lock was acquired
    pub acquired_at: Option<DateTime<Utc>>,

    /// Who acquired the lock (transaction ID, process ID, etc.)
    pub holder: Option<String>,

    /// Whether lock is currently held
    pub is_locked: bool,
}

impl OptimisticLock {
    /// Create a new optimistic lock
    pub fn new(credential_id: CredentialId, expected_version: u32) -> Self {
        Self {
            credential_id,
            expected_version,
            new_version: None,
            acquired_at: None,
            holder: None,
            is_locked: false,
        }
    }

    /// Attempt to acquire the lock
    ///
    /// # T092: Acquire Lock
    ///
    /// # Arguments
    ///
    /// * `holder` - Identifier of the lock holder (transaction ID, etc.)
    ///
    /// # Returns
    ///
    /// * `RotationResult<()>` - Ok if lock acquired, error if already locked
    pub fn acquire_lock(&mut self, holder: impl Into<String>) -> RotationResult<()> {
        if self.is_locked {
            return Err(super::error::RotationError::ConcurrentRotation {
                credential_id: self.credential_id.clone(),
            });
        }

        self.is_locked = true;
        self.holder = Some(holder.into());
        self.acquired_at = Some(Utc::now());

        Ok(())
    }

    /// Release the lock
    ///
    /// # T092: Release Lock
    pub fn release_lock(&mut self) {
        self.is_locked = false;
        self.holder = None;
    }

    /// Compare and swap version numbers
    ///
    /// # T093: Compare and Swap
    ///
    /// Checks that the current version matches expected version,
    /// and if so, updates to new version. This prevents lost updates from
    /// concurrent modifications **when combined with atomic database operations**.
    ///
    /// **Note**: This method itself is NOT atomic. Atomicity must be provided by
    /// the storage layer (e.g., database transaction with WHERE version = expected_version).
    ///
    /// # Arguments
    ///
    /// * `current_version` - The actual current version from storage
    /// * `new_version` - The version to update to
    ///
    /// # Returns
    ///
    /// * `RotationResult<()>` - Ok if swap succeeded, error if version mismatch
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Expected version 1, want to update to version 2
    /// lock.compare_and_swap(1, 2)?; // Success
    ///
    /// // But if someone else updated to version 2 already
    /// lock.compare_and_swap(1, 2)?; // Error: version mismatch
    /// ```
    pub fn compare_and_swap(
        &mut self,
        current_version: u32,
        new_version: u32,
    ) -> RotationResult<()> {
        if current_version != self.expected_version {
            return Err(super::error::RotationError::ConcurrentRotation {
                credential_id: self.credential_id.clone(),
            });
        }

        self.new_version = Some(new_version);
        self.expected_version = new_version; // Update for next CAS

        Ok(())
    }

    /// Check if lock is held
    pub fn is_held(&self) -> bool {
        self.is_locked
    }

    /// Get lock holder
    pub fn get_holder(&self) -> Option<&str> {
        self.holder.as_deref()
    }
}

/// Two-phase commit transaction phase
///
/// Tracks the current phase in a two-phase commit protocol for
/// atomic credential rotation operations.
///
/// # T086: Transaction Phase
///
/// # State Transitions
///
/// ```text
/// Preparing → Prepared → Committing → Committed (success)
///    ↓           ↓           ↓
/// Aborting ← Aborting ← Aborting (failure at any stage)
///    ↓
/// Aborted
/// ```
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::TransactionPhase;
///
/// let phase = TransactionPhase::Preparing;
/// assert!(phase.can_abort());
/// assert!(!phase.is_terminal());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionPhase {
    /// Preparing new credential (creating, generating secrets)
    Preparing,

    /// Preparation complete, ready to commit
    Prepared,

    /// Committing changes (atomic swap)
    Committing,

    /// Successfully committed
    Committed,

    /// Aborting transaction (cleanup in progress)
    Aborting,

    /// Transaction aborted and cleaned up
    Aborted,
}

impl TransactionPhase {
    /// Check if phase is terminal (Committed or Aborted)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TransactionPhase::Committed | TransactionPhase::Aborted
        )
    }

    /// Check if transaction can be aborted from current phase
    pub fn can_abort(&self) -> bool {
        !matches!(
            self,
            TransactionPhase::Committed | TransactionPhase::Aborting | TransactionPhase::Aborted
        )
    }

    /// Check if transaction can proceed to commit
    pub fn can_commit(&self) -> bool {
        matches!(self, TransactionPhase::Prepared)
    }

    /// Get next phase on successful progression
    pub fn next_phase(&self) -> Option<TransactionPhase> {
        match self {
            TransactionPhase::Preparing => Some(TransactionPhase::Prepared),
            TransactionPhase::Prepared => Some(TransactionPhase::Committing),
            TransactionPhase::Committing => Some(TransactionPhase::Committed),
            TransactionPhase::Aborting => Some(TransactionPhase::Aborted),
            TransactionPhase::Committed | TransactionPhase::Aborted => None,
        }
    }
}

/// Rollback strategy for failed rotations
///
/// Determines how to handle rotation failures.
///
/// # T072: Rollback Strategy
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::RollbackStrategy;
///
/// let strategy = RollbackStrategy::Automatic;
/// if strategy.should_rollback_automatically() {
///     // Perform automatic rollback
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RollbackStrategy {
    /// Automatically rollback on validation failure
    #[default]
    Automatic,

    /// Require manual intervention to rollback
    Manual,

    /// No rollback - leave in failed state
    None,
}

impl RollbackStrategy {
    /// Check if strategy allows automatic rollback
    pub fn should_rollback_automatically(&self) -> bool {
        matches!(self, RollbackStrategy::Automatic)
    }

    /// Check if manual intervention is required
    pub fn requires_manual_intervention(&self) -> bool {
        matches!(self, RollbackStrategy::Manual)
    }
}

/// Manual rotation metadata for emergency incident response
///
/// Tracks context for manual rotations including who triggered it and why.
/// This is crucial for audit trails during security incidents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualRotation {
    /// Reason for manual rotation (e.g., "Credential compromised in data breach")
    pub reason: String,

    /// Who triggered the rotation (username, service account, etc.)
    pub triggered_by: String,

    /// When the rotation was manually triggered
    pub triggered_at: DateTime<Utc>,

    /// Whether this is an emergency rotation (immediate revocation)
    pub is_emergency: bool,

    /// Incident tracking ID (optional, for linking to incident management systems)
    pub incident_id: Option<String>,
}

impl ManualRotation {
    /// Create manual rotation metadata
    ///
    /// # Arguments
    ///
    /// * `reason` - Why the rotation is being performed
    /// * `triggered_by` - Who initiated the rotation
    /// * `is_emergency` - Whether to immediately revoke old credential
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_credential::rotation::ManualRotation;
    ///
    /// let manual = ManualRotation::new(
    ///     "Suspected credential compromise detected in logs",
    ///     "security-team@example.com",
    ///     true, // Emergency - immediate revocation
    /// );
    /// ```
    pub fn new(
        reason: impl Into<String>,
        triggered_by: impl Into<String>,
        is_emergency: bool,
    ) -> Self {
        Self {
            reason: reason.into(),
            triggered_by: triggered_by.into(),
            triggered_at: Utc::now(),
            is_emergency,
            incident_id: None,
        }
    }

    /// Create emergency rotation (immediate revocation)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let emergency = ManualRotation::emergency(
    ///     "API key leaked in public GitHub repository",
    ///     "incident-response-bot",
    /// );
    /// ```
    pub fn emergency(reason: impl Into<String>, triggered_by: impl Into<String>) -> Self {
        Self::new(reason, triggered_by, true)
    }

    /// Create planned manual rotation (with grace period)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let planned = ManualRotation::planned(
    ///     "Regular security audit - rotating all database credentials",
    ///     "admin@example.com",
    /// );
    /// ```
    pub fn planned(reason: impl Into<String>, triggered_by: impl Into<String>) -> Self {
        Self::new(reason, triggered_by, false)
    }

    /// Link to incident tracking system
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut rotation = ManualRotation::emergency(
    ///     "Credential compromise",
    ///     "security-team",
    /// );
    /// rotation.link_incident("INC-2024-001");
    /// ```
    pub fn link_incident(&mut self, incident_id: impl Into<String>) {
        self.incident_id = Some(incident_id.into());
    }
}

impl BackupId {
    /// Generate a new backup ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the inner UUID
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Convert to string representation
    pub fn as_str(&self) -> String {
        self.0.to_string()
    }
}

impl Default for BackupId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for BackupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for BackupId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl RotationTransaction {
    /// Create a new rotation transaction
    pub fn new(credential_id: CredentialId, old_version: u32) -> Self {
        Self {
            id: RotationId::new(),
            credential_id,
            state: RotationState::Pending,
            old_version,
            new_version: None,
            started_at: Utc::now(),
            completed_at: None,
            validation_result: None,
            backup_id: None,
            grace_period_end: None,
            error_message: None,
            manual_rotation: None,
            rollback_strategy: RollbackStrategy::default(),
            transaction_phase: None,
            optimistic_lock: None,
        }
    }

    /// Create a new manual rotation transaction
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential to rotate
    /// * `old_version` - Current version number
    /// * `manual` - Manual rotation metadata (reason, triggered_by, etc.)
    pub fn new_manual(
        credential_id: CredentialId,
        old_version: u32,
        manual: ManualRotation,
    ) -> Self {
        Self {
            id: RotationId::new(),
            credential_id,
            state: RotationState::Pending,
            old_version,
            new_version: None,
            started_at: Utc::now(),
            completed_at: None,
            validation_result: None,
            backup_id: None,
            grace_period_end: None,
            error_message: None,
            manual_rotation: Some(manual),
            rollback_strategy: RollbackStrategy::default(),
            transaction_phase: None,
            optimistic_lock: None,
        }
    }

    /// Transition to a new state
    pub fn transition_to(&mut self, new_state: RotationState) -> RotationResult<()> {
        let validated_state = self.state.transition_to(new_state)?;
        self.state = validated_state;

        // Set completed timestamp for terminal states
        if self.state.is_terminal() && self.completed_at.is_none() {
            self.completed_at = Some(Utc::now());
        }

        Ok(())
    }

    /// Set the new credential version
    pub fn set_new_version(&mut self, version: u32) {
        self.new_version = Some(version);
    }

    /// Set validation result
    pub fn set_validation_result(&mut self, passed: bool, message: String) {
        self.validation_result = Some(ValidationResult {
            passed,
            message,
            validated_at: Utc::now(),
        });
    }

    /// Set backup ID
    pub fn set_backup_id(&mut self, backup_id: BackupId) {
        self.backup_id = Some(backup_id);
    }

    /// Set grace period end time
    pub fn set_grace_period_end(&mut self, end_time: DateTime<Utc>) {
        self.grace_period_end = Some(end_time);
    }

    /// Set error message
    pub fn set_error(&mut self, error: String) {
        self.error_message = Some(error);
    }

    /// Set rollback strategy
    pub fn set_rollback_strategy(&mut self, strategy: RollbackStrategy) {
        self.rollback_strategy = strategy;
    }

    /// Rollback the transaction to previous state
    ///
    /// # T073: Rollback Transaction
    ///
    /// Performs rollback by transitioning to RolledBack state and recording
    /// the error that triggered the rollback.
    ///
    /// # Arguments
    ///
    /// * `reason` - Why the rollback was triggered
    ///
    /// # Returns
    ///
    /// * `RotationResult<()>` - Ok if rollback succeeded
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_credential::rotation::RotationTransaction;
    ///
    /// let mut transaction = RotationTransaction::new(credential_id, 1);
    /// transaction.transition_to(RotationState::Validating)?;
    ///
    /// // Validation failed
    /// transaction.rollback_transaction("Validation failed: connection timeout")?;
    ///
    /// assert!(transaction.is_rolled_back());
    /// ```
    pub fn rollback_transaction(&mut self, reason: impl Into<String>) -> RotationResult<()> {
        use super::state::RotationState;

        let reason_str = reason.into();

        // Record error message
        self.set_error(reason_str.clone());

        // Transition to RolledBack state
        self.transition_to(RotationState::RolledBack)?;

        Ok(())
    }

    /// Check if rollback should be performed automatically
    pub fn should_auto_rollback(&self) -> bool {
        self.rollback_strategy.should_rollback_automatically()
    }

    /// Begin two-phase commit transaction
    ///
    /// # T087: Begin Transaction
    ///
    /// Initializes a two-phase commit protocol for atomic rotation.
    ///
    /// # Returns
    ///
    /// * `RotationResult<()>` - Ok if transaction started successfully
    pub fn begin_transaction(&mut self) -> RotationResult<()> {
        if self.transaction_phase.is_some() {
            return Err(super::error::RotationError::TransactionFailed {
                reason: "Transaction already in progress".to_string(),
            });
        }

        self.transaction_phase = Some(TransactionPhase::Preparing);
        Ok(())
    }

    /// Complete preparation phase
    ///
    /// # T088: Prepare Phase
    ///
    /// Marks that credential creation and validation are complete.
    /// Ready to proceed to commit phase.
    pub fn prepare_phase(&mut self) -> RotationResult<()> {
        match self.transaction_phase {
            Some(TransactionPhase::Preparing) => {
                self.transaction_phase = Some(TransactionPhase::Prepared);
                Ok(())
            }
            Some(phase) => Err(super::error::RotationError::InvalidStateTransition {
                from: format!("{:?}", phase),
                to: "Prepared".to_string(),
            }),
            None => Err(super::error::RotationError::TransactionFailed {
                reason: "No transaction in progress".to_string(),
            }),
        }
    }

    /// Execute commit phase (atomic swap)
    ///
    /// # T089: Commit Phase
    ///
    /// Performs atomic credential swap. This is the point of no return.
    pub fn commit_phase(&mut self) -> RotationResult<()> {
        match self.transaction_phase {
            Some(TransactionPhase::Prepared) => {
                self.transaction_phase = Some(TransactionPhase::Committing);
                Ok(())
            }
            Some(phase) => Err(super::error::RotationError::InvalidStateTransition {
                from: format!("{:?}", phase),
                to: "Committing".to_string(),
            }),
            None => Err(super::error::RotationError::TransactionFailed {
                reason: "No transaction in progress".to_string(),
            }),
        }
    }

    /// Complete commit phase
    pub fn complete_commit(&mut self) -> RotationResult<()> {
        match self.transaction_phase {
            Some(TransactionPhase::Committing) => {
                self.transaction_phase = Some(TransactionPhase::Committed);
                self.completed_at = Some(chrono::Utc::now());
                Ok(())
            }
            Some(phase) => Err(super::error::RotationError::InvalidStateTransition {
                from: format!("{:?}", phase),
                to: "Committed".to_string(),
            }),
            None => Err(super::error::RotationError::TransactionFailed {
                reason: "No transaction in progress".to_string(),
            }),
        }
    }

    /// Abort transaction and cleanup
    ///
    /// # T090: Abort Transaction
    ///
    /// Aborts the transaction and performs cleanup (delete new credential, restore state).
    ///
    /// # Arguments
    ///
    /// * `reason` - Why the transaction was aborted
    pub fn abort_transaction(&mut self, reason: impl Into<String>) -> RotationResult<()> {
        let reason_str = reason.into();

        match self.transaction_phase {
            Some(
                TransactionPhase::Preparing
                | TransactionPhase::Prepared
                | TransactionPhase::Committing,
            ) => {
                self.transaction_phase = Some(TransactionPhase::Aborting);
                self.set_error(reason_str);
                Ok(())
            }
            Some(TransactionPhase::Committed) => {
                Err(super::error::RotationError::TransactionFailed {
                    reason: "Cannot abort committed transaction".to_string(),
                })
            }
            Some(TransactionPhase::Aborting | TransactionPhase::Aborted) => Ok(()), // Already aborting
            None => Err(super::error::RotationError::TransactionFailed {
                reason: "No transaction in progress".to_string(),
            }),
        }
    }

    /// Complete abort phase
    pub fn complete_abort(&mut self) -> RotationResult<()> {
        match self.transaction_phase {
            Some(TransactionPhase::Aborting) => {
                self.transaction_phase = Some(TransactionPhase::Aborted);
                self.completed_at = Some(chrono::Utc::now());
                Ok(())
            }
            Some(phase) => Err(super::error::RotationError::InvalidStateTransition {
                from: format!("{:?}", phase),
                to: "Aborted".to_string(),
            }),
            None => Err(super::error::RotationError::TransactionFailed {
                reason: "No transaction in progress".to_string(),
            }),
        }
    }

    /// Check if using two-phase commit
    pub fn is_two_phase_commit(&self) -> bool {
        self.transaction_phase.is_some()
    }

    /// Check if rotation is complete
    pub fn is_complete(&self) -> bool {
        self.state.is_terminal()
    }

    /// Check if rotation succeeded
    pub fn is_successful(&self) -> bool {
        self.state.is_committed()
    }

    /// Check if rotation failed
    pub fn is_failed(&self) -> bool {
        self.state.is_rolled_back()
    }

    /// Mark rotation as creating
    pub fn mark_creating(&mut self) -> RotationResult<()> {
        self.transition_to(RotationState::Creating)
    }

    /// Mark rotation as validating
    pub fn mark_validating(&mut self) -> RotationResult<()> {
        self.transition_to(RotationState::Validating)
    }

    /// Mark rotation as committing
    pub fn mark_committing(&mut self) -> RotationResult<()> {
        self.transition_to(RotationState::Committing)
    }

    /// Mark rotation as committed
    pub fn mark_committed(&mut self) -> RotationResult<()> {
        self.transition_to(RotationState::Committed)
    }

    /// Mark rotation as rolled back
    pub fn mark_rolled_back(&mut self, reason: String) -> RotationResult<()> {
        self.set_error(reason);
        self.transition_to(RotationState::RolledBack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rotation_transaction_lifecycle() {
        let cred_id = CredentialId::new("test-credential").unwrap();
        let mut tx = RotationTransaction::new(cred_id, 1);

        // Initial state
        assert_eq!(tx.state, RotationState::Pending);
        assert!(!tx.is_complete());

        // Progress through states
        assert!(tx.mark_creating().is_ok());
        assert_eq!(tx.state, RotationState::Creating);

        tx.set_new_version(2);
        assert_eq!(tx.new_version, Some(2));

        assert!(tx.mark_validating().is_ok());
        assert_eq!(tx.state, RotationState::Validating);

        tx.set_validation_result(true, "Validation passed".to_string());
        assert!(tx.validation_result.is_some());
        assert!(tx.validation_result.as_ref().unwrap().passed);

        assert!(tx.mark_committing().is_ok());
        assert_eq!(tx.state, RotationState::Committing);

        assert!(tx.mark_committed().is_ok());
        assert_eq!(tx.state, RotationState::Committed);
        assert!(tx.is_complete());
        assert!(tx.is_successful());
        assert!(tx.completed_at.is_some());
    }

    #[test]
    fn test_rotation_rollback() {
        let cred_id = CredentialId::new("test-credential").unwrap();
        let mut tx = RotationTransaction::new(cred_id, 1);

        assert!(tx.mark_creating().is_ok());
        assert!(tx.mark_validating().is_ok());

        // Validation fails
        tx.set_validation_result(false, "Connection failed".to_string());
        assert!(tx.mark_rolled_back("Validation failed".to_string()).is_ok());

        assert_eq!(tx.state, RotationState::RolledBack);
        assert!(tx.is_complete());
        assert!(tx.is_failed());
        assert!(tx.error_message.is_some());
    }

    #[test]
    fn test_invalid_state_transition() {
        let cred_id = CredentialId::new("test-credential").unwrap();
        let mut tx = RotationTransaction::new(cred_id, 1);

        // Cannot skip states
        let result = tx.mark_committed();
        assert!(result.is_err());
    }

    #[test]
    fn test_rollback_strategy_automatic() {
        let strategy = RollbackStrategy::Automatic;
        assert!(strategy.should_rollback_automatically());
        assert!(!strategy.requires_manual_intervention());
    }

    #[test]
    fn test_rollback_strategy_manual() {
        let strategy = RollbackStrategy::Manual;
        assert!(!strategy.should_rollback_automatically());
        assert!(strategy.requires_manual_intervention());
    }

    #[test]
    fn test_rollback_strategy_none() {
        let strategy = RollbackStrategy::None;
        assert!(!strategy.should_rollback_automatically());
        assert!(!strategy.requires_manual_intervention());
    }

    #[test]
    fn test_rollback_transaction_method() {
        let cred_id = CredentialId::new("test-credential").unwrap();
        let mut tx = RotationTransaction::new(cred_id, 1);

        // Progress to validating state
        tx.mark_creating().unwrap();
        tx.mark_validating().unwrap();

        // Rollback due to validation failure
        let result = tx.rollback_transaction("Validation failed: connection timeout");
        assert!(result.is_ok());

        assert_eq!(tx.state, RotationState::RolledBack);
        assert!(tx.is_failed());
        assert!(tx.is_complete());
        assert_eq!(
            tx.error_message.as_deref(),
            Some("Validation failed: connection timeout")
        );
    }

    #[test]
    fn test_should_auto_rollback() {
        let cred_id = CredentialId::new("test-credential").unwrap();
        let mut tx = RotationTransaction::new(cred_id, 1);

        // Default is Automatic
        assert!(tx.should_auto_rollback());

        // Change to Manual
        tx.set_rollback_strategy(RollbackStrategy::Manual);
        assert!(!tx.should_auto_rollback());

        // Change to None
        tx.set_rollback_strategy(RollbackStrategy::None);
        assert!(!tx.should_auto_rollback());
    }
}
