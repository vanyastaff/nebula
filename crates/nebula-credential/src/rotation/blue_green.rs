//! Blue-Green Deployment Pattern for Zero-Downtime Rotation
//!
//! Implements blue-green credential rotation for databases and other critical services.
//! This pattern ensures zero downtime by:
//! 1. Creating a new "standby" credential (green) alongside the active one (blue)
//! 2. Validating the standby credential works correctly
//! 3. Atomically swapping active/standby credentials
//! 4. Keeping the old credential as standby for quick rollback

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::{RotationError, RotationResult};
use crate::core::CredentialId;

/// Blue-Green rotation state
///
/// Tracks the current state of a blue-green credential rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueGreenState {
    /// Blue (current active) credential is in use
    Blue,

    /// Green (new standby) credential is in use
    Green,

    /// Currently transitioning between blue and green
    Transitioning,

    /// Rolled back to previous state
    RolledBack,
}

impl BlueGreenState {
    /// Check if in a stable state (Blue or Green)
    pub fn is_stable(&self) -> bool {
        matches!(self, BlueGreenState::Blue | BlueGreenState::Green)
    }

    /// Check if currently transitioning
    pub fn is_transitioning(&self) -> bool {
        matches!(self, BlueGreenState::Transitioning)
    }

    /// Get the next state after successful swap
    pub fn next_state(&self) -> RotationResult<BlueGreenState> {
        match self {
            BlueGreenState::Blue => Ok(BlueGreenState::Green),
            BlueGreenState::Green => Ok(BlueGreenState::Blue),
            BlueGreenState::Transitioning => Err(RotationError::InvalidStateTransition {
                from: "Transitioning".to_string(),
                to: "Cannot determine next state while transitioning".to_string(),
            }),
            BlueGreenState::RolledBack => Err(RotationError::InvalidStateTransition {
                from: "RolledBack".to_string(),
                to: "Cannot transition from rolled back state".to_string(),
            }),
        }
    }
}

/// Blue-Green rotation tracker
///
/// Maintains state for blue-green credential rotation with active/standby tracking.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::blue_green::{BlueGreenRotation, BlueGreenState};
///
/// let rotation = BlueGreenRotation::new(
///     blue_id.clone(),
///     green_id.clone(),
/// );
///
/// // Validate standby credential
/// rotation.validate_standby().await?;
///
/// // Swap to new credential
/// rotation.swap().await?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueGreenRotation {
    /// Current state
    pub state: BlueGreenState,

    /// Blue credential ID (initial active)
    pub blue_credential: CredentialId,

    /// Green credential ID (standby/new)
    pub green_credential: CredentialId,

    /// Currently active credential
    pub active_credential: CredentialId,

    /// Currently standby credential
    pub standby_credential: CredentialId,

    /// When rotation was initiated
    pub started_at: DateTime<Utc>,

    /// When last state transition occurred
    pub last_transition: Option<DateTime<Utc>>,

    /// Whether standby has been validated
    pub standby_validated: bool,
}

impl BlueGreenRotation {
    /// Create a new blue-green rotation
    ///
    /// # Arguments
    ///
    /// * `blue_credential` - Current active credential (blue)
    /// * `green_credential` - New standby credential (green)
    pub fn new(blue_credential: CredentialId, green_credential: CredentialId) -> Self {
        Self {
            state: BlueGreenState::Blue,
            blue_credential: blue_credential.clone(),
            green_credential: green_credential.clone(),
            active_credential: blue_credential,
            standby_credential: green_credential,
            started_at: Utc::now(),
            last_transition: None,
            standby_validated: false,
        }
    }

    /// Mark standby credential as validated
    pub fn mark_validated(&mut self) {
        self.standby_validated = true;
    }

    /// Start transition to swap credentials
    pub fn start_transition(&mut self) -> RotationResult<()> {
        if !self.standby_validated {
            return Err(RotationError::ValidationFailed {
                credential_id: self.standby_credential.clone(),
                reason: "Standby credential not validated before swap".to_string(),
            });
        }

        if self.state.is_transitioning() {
            return Err(RotationError::InvalidStateTransition {
                from: "Transitioning".to_string(),
                to: "Already in transitioning state".to_string(),
            });
        }

        self.state = BlueGreenState::Transitioning;
        self.last_transition = Some(Utc::now());
        Ok(())
    }

    /// Complete the swap to standby credential
    pub fn complete_swap(&mut self) -> RotationResult<()> {
        if !self.state.is_transitioning() {
            return Err(RotationError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "Can only complete swap from Transitioning state".to_string(),
            });
        }

        // Swap active and standby
        std::mem::swap(&mut self.active_credential, &mut self.standby_credential);

        // Update state to the next stable state
        self.state = if self.active_credential == self.blue_credential {
            BlueGreenState::Blue
        } else {
            BlueGreenState::Green
        };

        self.last_transition = Some(Utc::now());
        self.standby_validated = false; // Reset for next rotation

        Ok(())
    }

    /// Rollback the swap
    pub fn rollback(&mut self) -> RotationResult<()> {
        if !self.state.is_transitioning() {
            return Err(RotationError::RollbackFailed {
                credential_id: self.active_credential.clone(),
                reason: "Can only rollback from Transitioning state".to_string(),
            });
        }

        // Restore previous state
        self.state = if self.active_credential == self.blue_credential {
            BlueGreenState::Blue
        } else {
            BlueGreenState::Green
        };

        self.last_transition = Some(Utc::now());

        Ok(())
    }

    /// Get the currently active credential ID
    pub fn active(&self) -> &CredentialId {
        &self.active_credential
    }

    /// Get the currently standby credential ID
    pub fn standby(&self) -> &CredentialId {
        &self.standby_credential
    }

    /// Create standby credential with mirrored privileges
    ///
    /// Generates a new credential that mirrors the active credential's configuration
    /// but with a different identifier and secret.
    ///
    /// # T057: Standby Creation
    pub async fn create_standby_credential<F, Fut>(
        &self,
        credential_factory: F,
    ) -> RotationResult<()>
    where
        F: FnOnce(CredentialId) -> Fut,
        Fut: std::future::Future<Output = RotationResult<()>>,
    {
        if self.standby_validated {
            return Err(RotationError::ValidationFailed {
                credential_id: self.standby_credential.clone(),
                reason: "Standby credential already exists and validated".to_string(),
            });
        }

        // Invoke the factory to create standby credential
        credential_factory(self.standby_credential.clone()).await?;

        Ok(())
    }

    /// Validate standby credential connectivity
    ///
    /// Tests that the standby credential can successfully connect and perform
    /// basic operations before swapping to it.
    ///
    /// # T058: Connectivity Validation
    pub async fn validate_standby_connectivity<F, Fut>(
        &mut self,
        connectivity_test: F,
    ) -> RotationResult<()>
    where
        F: FnOnce(CredentialId) -> Fut,
        Fut: std::future::Future<Output = RotationResult<()>>,
    {
        if self.standby_validated {
            return Ok(()); // Already validated
        }

        // Run connectivity test
        connectivity_test(self.standby_credential.clone()).await?;

        // Mark as validated on success
        self.mark_validated();

        Ok(())
    }

    /// Atomically swap active and standby credentials
    ///
    /// Performs the blue-green swap with validation checks and rollback capability.
    ///
    /// # T059: Atomic Swap
    pub async fn swap_credentials<F, Fut>(&mut self, swap_operation: F) -> RotationResult<()>
    where
        F: FnOnce(CredentialId, CredentialId) -> Fut,
        Fut: std::future::Future<Output = RotationResult<()>>,
    {
        // Start transition
        self.start_transition()?;

        // Attempt swap operation
        match swap_operation(
            self.active_credential.clone(),
            self.standby_credential.clone(),
        )
        .await
        {
            Ok(()) => {
                // Complete swap on success
                self.complete_swap()?;
                Ok(())
            }
            Err(e) => {
                // Rollback on failure
                self.rollback()?;
                Err(e)
            }
        }
    }
}

/// Database privilege types
///
/// Common database permissions for credential validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DatabasePrivilege {
    /// Connect to database
    Connect,

    /// SELECT queries
    Select,

    /// INSERT operations
    Insert,

    /// UPDATE operations
    Update,

    /// DELETE operations
    Delete,

    /// CREATE operations
    Create,

    /// DROP operations
    Drop,

    /// ALTER operations
    Alter,

    /// All privileges
    All,
}

impl DatabasePrivilege {
    /// Get all standard privileges (excluding All)
    pub fn all_standard() -> Vec<DatabasePrivilege> {
        vec![
            DatabasePrivilege::Connect,
            DatabasePrivilege::Select,
            DatabasePrivilege::Insert,
            DatabasePrivilege::Update,
            DatabasePrivilege::Delete,
            DatabasePrivilege::Create,
            DatabasePrivilege::Drop,
            DatabasePrivilege::Alter,
        ]
    }

    /// Get read-only privileges
    pub fn read_only() -> Vec<DatabasePrivilege> {
        vec![DatabasePrivilege::Connect, DatabasePrivilege::Select]
    }

    /// Get read-write privileges (no DDL)
    pub fn read_write() -> Vec<DatabasePrivilege> {
        vec![
            DatabasePrivilege::Connect,
            DatabasePrivilege::Select,
            DatabasePrivilege::Insert,
            DatabasePrivilege::Update,
            DatabasePrivilege::Delete,
        ]
    }
}

impl std::fmt::Display for DatabasePrivilege {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabasePrivilege::Connect => write!(f, "CONNECT"),
            DatabasePrivilege::Select => write!(f, "SELECT"),
            DatabasePrivilege::Insert => write!(f, "INSERT"),
            DatabasePrivilege::Update => write!(f, "UPDATE"),
            DatabasePrivilege::Delete => write!(f, "DELETE"),
            DatabasePrivilege::Create => write!(f, "CREATE"),
            DatabasePrivilege::Drop => write!(f, "DROP"),
            DatabasePrivilege::Alter => write!(f, "ALTER"),
            DatabasePrivilege::All => write!(f, "ALL"),
        }
    }
}

/// Enumerate required privileges for a credential
///
/// Returns the set of database privileges that must be present for the
/// credential to function correctly.
///
/// # T061: Privilege Enumeration
pub async fn enumerate_required_privileges<F, Fut>(
    credential_id: &CredentialId,
    privilege_enumerator: F,
) -> RotationResult<Vec<DatabasePrivilege>>
where
    F: FnOnce(CredentialId) -> Fut,
    Fut: std::future::Future<Output = RotationResult<Vec<DatabasePrivilege>>>,
{
    privilege_enumerator(credential_id.clone()).await
}

/// Validate that a credential has all required privileges
///
/// Checks that the standby credential has the same privileges as the active
/// credential before performing the swap.
///
/// # T062: Privilege Validation
pub async fn validate_privileges<F, Fut>(
    credential_id: &CredentialId,
    required_privileges: &[DatabasePrivilege],
    privilege_validator: F,
) -> RotationResult<()>
where
    F: FnOnce(CredentialId, Vec<DatabasePrivilege>) -> Fut,
    Fut: std::future::Future<Output = RotationResult<Vec<DatabasePrivilege>>>,
{
    // Get actual privileges
    let actual_privileges =
        privilege_validator(credential_id.clone(), required_privileges.to_vec()).await?;

    // Check if all required privileges are present
    let missing: Vec<_> = required_privileges
        .iter()
        .filter(|&req| !actual_privileges.contains(req))
        .collect();

    if !missing.is_empty() {
        return Err(RotationError::ValidationFailed {
            credential_id: credential_id.clone(),
            reason: format!(
                "Missing required privileges: {}",
                missing
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blue_green_state_transitions() {
        assert!(BlueGreenState::Blue.is_stable());
        assert!(BlueGreenState::Green.is_stable());
        assert!(!BlueGreenState::Transitioning.is_stable());
        assert!(!BlueGreenState::RolledBack.is_stable());

        assert_eq!(
            BlueGreenState::Blue.next_state().unwrap(),
            BlueGreenState::Green
        );
        assert_eq!(
            BlueGreenState::Green.next_state().unwrap(),
            BlueGreenState::Blue
        );
    }

    #[test]
    fn test_blue_green_rotation_creation() {
        let blue = CredentialId::new("blue-cred").unwrap();
        let green = CredentialId::new("green-cred").unwrap();

        let rotation = BlueGreenRotation::new(blue.clone(), green.clone());

        assert_eq!(rotation.state, BlueGreenState::Blue);
        assert_eq!(rotation.active(), &blue);
        assert_eq!(rotation.standby(), &green);
        assert!(!rotation.standby_validated);
    }

    #[test]
    fn test_blue_green_rotation_swap() {
        let blue = CredentialId::new("blue-cred").unwrap();
        let green = CredentialId::new("green-cred").unwrap();

        let mut rotation = BlueGreenRotation::new(blue.clone(), green.clone());

        // Can't transition without validation
        assert!(rotation.start_transition().is_err());

        // Mark as validated
        rotation.mark_validated();
        assert!(rotation.standby_validated);

        // Start transition
        assert!(rotation.start_transition().is_ok());
        assert_eq!(rotation.state, BlueGreenState::Transitioning);

        // Complete swap
        assert!(rotation.complete_swap().is_ok());
        assert_eq!(rotation.state, BlueGreenState::Green);
        assert_eq!(rotation.active(), &green);
        assert_eq!(rotation.standby(), &blue);
        assert!(!rotation.standby_validated); // Reset after swap
    }

    #[test]
    fn test_blue_green_rollback() {
        let blue = CredentialId::new("blue-cred").unwrap();
        let green = CredentialId::new("green-cred").unwrap();

        let mut rotation = BlueGreenRotation::new(blue.clone(), green.clone());
        rotation.mark_validated();
        rotation.start_transition().unwrap();

        // Rollback during transition
        assert!(rotation.rollback().is_ok());
        assert_eq!(rotation.state, BlueGreenState::Blue);
        assert_eq!(rotation.active(), &blue);
    }

    #[test]
    fn test_database_privileges() {
        assert_eq!(DatabasePrivilege::all_standard().len(), 8);
        assert_eq!(DatabasePrivilege::read_only().len(), 2);
        assert_eq!(DatabasePrivilege::read_write().len(), 5);

        assert_eq!(DatabasePrivilege::Select.to_string(), "SELECT");
        assert_eq!(DatabasePrivilege::All.to_string(), "ALL");
    }
}
