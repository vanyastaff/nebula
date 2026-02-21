//! Persistent trigger state

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Persistent state for a webhook trigger
///
/// This state is stored across restarts to maintain stable webhook URLs.
/// Each trigger has two UUIDs - one for test environment and one for production.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerState {
    /// Unique identifier for this trigger within the workflow
    pub trigger_id: String,

    /// UUID for test environment webhook path
    ///
    /// Generated once and never changes, ensuring stable test URLs.
    pub test_uuid: Uuid,

    /// UUID for production environment webhook path
    ///
    /// Generated once and never changes, ensuring stable production URLs.
    pub prod_uuid: Uuid,

    /// When this trigger state was created
    pub created_at: DateTime<Utc>,

    /// Last time this trigger was subscribed (registered with provider)
    pub last_subscribed: Option<DateTime<Utc>>,

    /// Last time this trigger received a webhook
    pub last_webhook: Option<DateTime<Utc>>,

    /// Additional metadata for the trigger
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

impl TriggerState {
    /// Create a new trigger state with generated UUIDs
    ///
    /// # Arguments
    ///
    /// * `trigger_id` - Unique identifier for this trigger
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_webhook::TriggerState;
    ///
    /// let state = TriggerState::new("my-trigger");
    /// assert!(!state.test_uuid.is_nil());
    /// assert!(!state.prod_uuid.is_nil());
    /// assert_ne!(state.test_uuid, state.prod_uuid);
    /// ```
    pub fn new(trigger_id: impl Into<String>) -> Self {
        Self {
            trigger_id: trigger_id.into(),
            test_uuid: Uuid::new_v4(),
            prod_uuid: Uuid::new_v4(),
            created_at: Utc::now(),
            last_subscribed: None,
            last_webhook: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Create or restore trigger state from storage
    ///
    /// This method checks if state exists in storage and restores it,
    /// or creates new state if not found. This ensures stable UUIDs
    /// across pod restarts in Kubernetes.
    ///
    /// # Arguments
    ///
    /// * `trigger_id` - Unique identifier for this trigger
    /// * `store` - Optional state store for persistence
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nebula_webhook::TriggerState;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Without store - creates new state every time
    /// let state1 = TriggerState::new_or_restore("my-trigger", None).await?;
    ///
    /// // With store - restores from storage if exists
    /// // let store = Arc::new(RedisStateStore::new(...));
    /// // let state2 = TriggerState::new_or_restore("my-trigger", Some(store)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new_or_restore(
        trigger_id: impl Into<String>,
        _store: Option<std::sync::Arc<dyn crate::store::StateStore>>,
    ) -> crate::Result<Self> {
        let trigger_id = trigger_id.into();

        // TODO: Load from store if provided
        // if let Some(store) = store {
        //     if let Some(state) = store.load(&trigger_id).await? {
        //         return Ok(state);
        //     }
        // }

        // Create new state if not found
        Ok(Self::new(trigger_id))
    }

    /// Create a trigger state with specific UUIDs (for deserialization)
    pub fn with_uuids(trigger_id: impl Into<String>, test_uuid: Uuid, prod_uuid: Uuid) -> Self {
        Self {
            trigger_id: trigger_id.into(),
            test_uuid,
            prod_uuid,
            created_at: Utc::now(),
            last_subscribed: None,
            last_webhook: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Update the last subscribed timestamp
    pub fn mark_subscribed(&mut self) {
        self.last_subscribed = Some(Utc::now());
    }

    /// Update the last webhook timestamp
    pub fn mark_webhook_received(&mut self) {
        self.last_webhook = Some(Utc::now());
    }

    /// Add or update metadata
    pub fn set_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }

    /// Get metadata value
    pub fn get_metadata(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(String::as_str)
    }

    /// Get the UUID for the specified environment
    pub fn uuid_for_env(&self, env: &crate::Environment) -> Uuid {
        match env {
            crate::Environment::Test => self.test_uuid,
            crate::Environment::Production => self.prod_uuid,
        }
    }

    /// Get the age of this trigger state
    pub fn age(&self) -> chrono::Duration {
        Utc::now().signed_duration_since(self.created_at)
    }

    /// Check if the trigger has been subscribed
    pub fn is_subscribed(&self) -> bool {
        self.last_subscribed.is_some()
    }

    /// Check if the trigger has received webhooks
    pub fn has_received_webhooks(&self) -> bool {
        self.last_webhook.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_trigger_state() {
        let state = TriggerState::new("test-trigger");

        assert_eq!(state.trigger_id, "test-trigger");
        assert!(!state.test_uuid.is_nil());
        assert!(!state.prod_uuid.is_nil());
        assert_ne!(state.test_uuid, state.prod_uuid);
        assert!(!state.is_subscribed());
        assert!(!state.has_received_webhooks());
    }

    #[test]
    fn test_mark_subscribed() {
        let mut state = TriggerState::new("test-trigger");
        assert!(!state.is_subscribed());

        state.mark_subscribed();
        assert!(state.is_subscribed());
        assert!(state.last_subscribed.is_some());
    }

    #[test]
    fn test_mark_webhook_received() {
        let mut state = TriggerState::new("test-trigger");
        assert!(!state.has_received_webhooks());

        state.mark_webhook_received();
        assert!(state.has_received_webhooks());
        assert!(state.last_webhook.is_some());
    }

    #[test]
    fn test_metadata() {
        let mut state = TriggerState::new("test-trigger");

        state.set_metadata("key1", "value1");
        state.set_metadata("key2", "value2");

        assert_eq!(state.get_metadata("key1"), Some("value1"));
        assert_eq!(state.get_metadata("key2"), Some("value2"));
        assert_eq!(state.get_metadata("key3"), None);
    }

    #[test]
    fn test_uuid_for_env() {
        use crate::Environment;

        let state = TriggerState::new("test-trigger");
        let test_uuid = state.uuid_for_env(&Environment::Test);
        let prod_uuid = state.uuid_for_env(&Environment::Production);

        assert_eq!(test_uuid, state.test_uuid);
        assert_eq!(prod_uuid, state.prod_uuid);
        assert_ne!(test_uuid, prod_uuid);
    }

    #[test]
    fn test_serde() {
        let state = TriggerState::new("test-trigger");
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: TriggerState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.trigger_id, state.trigger_id);
        assert_eq!(deserialized.test_uuid, state.test_uuid);
        assert_eq!(deserialized.prod_uuid, state.prod_uuid);
    }

    #[test]
    fn test_age() {
        let state = TriggerState::new("test-trigger");
        let age = state.age();

        // Age should be very small (just created)
        assert!(age.num_seconds() < 5);
    }
}
