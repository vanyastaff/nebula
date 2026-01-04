//! Parameter context for runtime state management

use std::collections::HashMap;
use std::sync::Arc;

use nebula_core::ParameterKey;
use nebula_validator::core::ValidationError;
use nebula_value::Value;
use tokio::sync::broadcast;

use super::{
    DisplayContext, ParameterCollection, ParameterEvent, ParameterSnapshot, ParameterState,
    ParameterValues, StateFlags,
};

/// Default capacity for the event broadcast channel.
const DEFAULT_CHANNEL_CAPACITY: usize = 64;

/// Runtime context for parameter management.
///
/// Combines parameter definitions (schema), current values, and state tracking
/// with event broadcasting for reactive updates.
pub struct ParameterContext {
    /// Parameter definitions (schema).
    collection: Arc<ParameterCollection>,
    /// Current parameter values.
    values: ParameterValues,
    /// State for each parameter.
    states: HashMap<ParameterKey, ParameterState>,
    /// Event broadcaster.
    event_tx: broadcast::Sender<ParameterEvent>,
}

impl ParameterContext {
    /// Create a new context with empty values.
    #[must_use]
    pub fn new(collection: Arc<ParameterCollection>) -> Self {
        Self::with_channel_capacity(collection, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new context with specified channel capacity.
    #[must_use]
    pub fn with_channel_capacity(collection: Arc<ParameterCollection>, capacity: usize) -> Self {
        let (event_tx, _) = broadcast::channel(capacity);
        let mut states = HashMap::new();

        // Initialize state for each parameter
        for key in collection.keys() {
            states.insert(key.clone(), ParameterState::new());
        }

        Self {
            collection,
            values: ParameterValues::new(),
            states,
            event_tx,
        }
    }

    /// Create a context with initial values (e.g., loaded from database).
    #[must_use]
    pub fn with_values(collection: Arc<ParameterCollection>, values: ParameterValues) -> Self {
        let mut ctx = Self::new(collection);
        ctx.load(values);
        ctx
    }

    /// Subscribe to parameter events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ParameterEvent> {
        self.event_tx.subscribe()
    }

    /// Get a reference to the parameter collection (schema).
    #[must_use]
    pub fn collection(&self) -> &ParameterCollection {
        &self.collection
    }

    /// Get a reference to all current values.
    #[must_use]
    pub fn values(&self) -> &ParameterValues {
        &self.values
    }

    /// Get a specific parameter value.
    #[must_use]
    pub fn get(&self, key: &ParameterKey) -> Option<&Value> {
        self.values.get(key.clone())
    }

    /// Get a parameter's state.
    #[must_use]
    pub fn state(&self, key: &ParameterKey) -> Option<&ParameterState> {
        self.states.get(key)
    }

    /// Get mutable access to a parameter's state.
    pub fn state_mut(&mut self, key: &ParameterKey) -> Option<&mut ParameterState> {
        self.states.get_mut(key)
    }

    /// Set a parameter value.
    ///
    /// This will:
    /// - Update the value
    /// - Mark the parameter as dirty and touched
    /// - Emit a `ValueChanged` event
    pub fn set(&mut self, key: ParameterKey, value: Value) {
        let old = self.values.get(key.clone()).cloned().unwrap_or(Value::Null);

        // Don't emit event if value hasn't changed
        if old == value {
            return;
        }

        self.values.set(key.clone(), value.clone());

        // Update state
        if let Some(state) = self.states.get_mut(&key) {
            state.mark_dirty();
            state.mark_touched();
        }

        // Emit event
        let _ = self.event_tx.send(ParameterEvent::ValueChanged {
            key,
            old,
            new: value,
        });
    }

    /// Load values (replaces all current values).
    ///
    /// This is intended for initial load or reset, and marks all parameters as clean.
    /// Emits a `Loaded` event.
    pub fn load(&mut self, values: ParameterValues) {
        self.values = values;

        // Reset all states to clean
        for state in self.states.values_mut() {
            state.mark_clean();
            state.clear_flag(StateFlags::TOUCHED);
        }

        let _ = self.event_tx.send(ParameterEvent::Loaded);
    }

    /// Clear all values and reset states.
    ///
    /// Emits a `Cleared` event.
    pub fn clear(&mut self) {
        self.values.clear();

        for state in self.states.values_mut() {
            *state = ParameterState::new();
        }

        let _ = self.event_tx.send(ParameterEvent::Cleared);
    }

    /// Check if any parameter is dirty.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.states.values().any(|s| s.is_dirty())
    }

    /// Get all dirty parameter keys.
    #[must_use]
    pub fn dirty_keys(&self) -> Vec<ParameterKey> {
        self.states
            .iter()
            .filter(|(_, s)| s.is_dirty())
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Mark all parameters as clean.
    pub fn mark_all_clean(&mut self) {
        for state in self.states.values_mut() {
            state.mark_clean();
        }
    }

    /// Check if a parameter is visible.
    #[must_use]
    pub fn is_visible(&self, key: &ParameterKey) -> bool {
        self.states
            .get(key)
            .map(|s| s.is_visible())
            .unwrap_or(false)
    }

    /// Set parameter visibility.
    ///
    /// Emits a `VisibilityChanged` event if the visibility actually changed.
    pub fn set_visible(&mut self, key: ParameterKey, visible: bool) {
        if let Some(state) = self.states.get_mut(&key) {
            let was_visible = state.is_visible();
            if was_visible != visible {
                state.set_visible(visible);
                let _ = self
                    .event_tx
                    .send(ParameterEvent::VisibilityChanged { key, visible });
            }
        }
    }

    /// Get a snapshot of current values for saving.
    #[must_use]
    pub fn snapshot(&self) -> ParameterSnapshot {
        self.values.snapshot()
    }

    /// Restore from a snapshot.
    pub fn restore(&mut self, snapshot: &ParameterSnapshot) {
        self.values.restore(snapshot);

        // Mark all as clean after restore
        for state in self.states.values_mut() {
            state.mark_clean();
        }

        let _ = self.event_tx.send(ParameterEvent::Loaded);
    }

    // =========================================================================
    // Validation
    // =========================================================================

    /// Validate a single parameter.
    ///
    /// Updates the parameter's state with validation results and emits a `Validated` event.
    pub async fn validate(&mut self, key: &ParameterKey) -> Result<(), Vec<ValidationError>> {
        let value = self.values.get(key.clone()).cloned().unwrap_or(Value::Null);

        // Get parameter and validate
        let errors = if let Some(param) = self.collection.get_validatable(key.clone()) {
            match param.validate(&value).await {
                Ok(()) => Vec::new(),
                Err(e) => vec![ValidationError::new("validation_failed", e.to_string())],
            }
        } else {
            Vec::new()
        };

        // Update state
        if let Some(state) = self.states.get_mut(key) {
            state.set_errors(errors.clone());
        }

        // Emit event
        let _ = self.event_tx.send(ParameterEvent::Validated {
            key: key.clone(),
            errors: errors.clone(),
        });

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate all parameters.
    ///
    /// Returns a map of parameter keys to their validation errors.
    pub async fn validate_all(&mut self) -> HashMap<ParameterKey, Vec<ValidationError>> {
        let mut all_errors = HashMap::new();
        let keys: Vec<_> = self.states.keys().cloned().collect();

        for key in keys {
            if let Err(errors) = self.validate(&key).await {
                all_errors.insert(key, errors);
            }
        }

        all_errors
    }

    /// Check if all parameters are valid.
    #[must_use]
    pub fn is_all_valid(&self) -> bool {
        self.states.values().all(|s| s.is_valid())
    }

    /// Get all validation errors.
    #[must_use]
    pub fn all_errors(&self) -> HashMap<ParameterKey, Vec<ValidationError>> {
        self.states
            .iter()
            .filter(|(_, s)| !s.errors().is_empty())
            .map(|(k, s)| (k.clone(), s.errors().to_vec()))
            .collect()
    }

    // =========================================================================
    // Display Conditions
    // =========================================================================

    /// Create a DisplayContext from current state.
    #[must_use]
    pub fn display_context(&self) -> DisplayContext {
        let mut ctx = DisplayContext::from_values(self.values.clone());

        // Add validation states
        for (key, state) in &self.states {
            ctx = ctx.with_validation(key.clone(), state.is_valid());
        }

        ctx
    }

    /// Update visibility for a single parameter based on display conditions.
    ///
    /// Returns true if visibility changed.
    pub fn update_visibility(&mut self, key: &ParameterKey) -> bool {
        let ctx = self.display_context();

        let should_show = self
            .collection
            .get_displayable(key.clone())
            .map(|p| p.should_display(&ctx))
            .unwrap_or(true);

        let state = match self.states.get_mut(key) {
            Some(s) => s,
            None => return false,
        };

        let was_visible = state.is_visible();
        if was_visible != should_show {
            state.set_visible(should_show);
            let _ = self.event_tx.send(ParameterEvent::VisibilityChanged {
                key: key.clone(),
                visible: should_show,
            });
            true
        } else {
            false
        }
    }

    /// Update visibility for all parameters.
    ///
    /// Returns list of keys whose visibility changed.
    pub fn update_all_visibility(&mut self) -> Vec<ParameterKey> {
        let keys: Vec<_> = self.states.keys().cloned().collect();
        let mut changed = Vec::new();

        for key in keys {
            if self.update_visibility(&key) {
                changed.push(key);
            }
        }

        changed
    }

    /// Update visibility for dependents of a parameter.
    ///
    /// Call this after a value changes to update parameters that depend on it.
    pub fn update_dependent_visibility(&mut self, changed_key: &ParameterKey) -> Vec<ParameterKey> {
        let dependents = self.collection.get_dependents(changed_key.clone());
        let mut changed = Vec::new();

        for key in dependents {
            if self.update_visibility(&key) {
                changed.push(key);
            }
        }

        changed
    }

    // =========================================================================
    // Reactive Operations
    // =========================================================================

    /// Set a value with full reactive updates.
    ///
    /// This will:
    /// 1. Update the value
    /// 2. Mark as dirty/touched
    /// 3. Validate the parameter
    /// 4. Update visibility of dependent parameters
    /// 5. Emit all relevant events
    pub async fn set_reactive(&mut self, key: ParameterKey, value: Value) {
        let old = self.values.get(key.clone()).cloned().unwrap_or(Value::Null);

        // Don't do anything if value hasn't changed
        if old == value {
            return;
        }

        // 1. Update value
        self.values.set(key.clone(), value.clone());

        // 2. Update state flags
        if let Some(state) = self.states.get_mut(&key) {
            state.mark_dirty();
            state.mark_touched();
        }

        // 3. Emit value changed event
        let _ = self.event_tx.send(ParameterEvent::ValueChanged {
            key: key.clone(),
            old,
            new: value,
        });

        // 4. Validate the changed parameter
        let _ = self.validate(&key).await;

        // 5. Update visibility of dependent parameters
        self.update_dependent_visibility(&key);
    }
}

impl std::fmt::Debug for ParameterContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterContext")
            .field("values_count", &self.values.len())
            .field("states_count", &self.states.len())
            .field("is_dirty", &self.is_dirty())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TextParameter;

    fn test_key(name: &str) -> ParameterKey {
        ParameterKey::new(name).unwrap()
    }

    fn create_test_collection() -> Arc<ParameterCollection> {
        let mut collection = ParameterCollection::new();
        collection.add(
            TextParameter::builder()
                .key("name")
                .name("Name")
                .build()
                .unwrap(),
        );
        collection.add(
            TextParameter::builder()
                .key("email")
                .name("Email")
                .build()
                .unwrap(),
        );
        Arc::new(collection)
    }

    #[test]
    fn test_new_context() {
        let collection = create_test_collection();
        let ctx = ParameterContext::new(collection);

        assert!(!ctx.is_dirty());
        assert!(ctx.values().is_empty());
    }

    #[test]
    fn test_set_value() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        ctx.set(test_key("name"), Value::text("Alice"));

        assert_eq!(ctx.get(&test_key("name")), Some(&Value::text("Alice")));
        assert!(ctx.is_dirty());
        assert!(ctx.state(&test_key("name")).unwrap().is_touched());
    }

    #[test]
    fn test_load_values() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        let mut values = ParameterValues::new();
        values.set(test_key("name"), Value::text("Bob"));
        values.set(test_key("email"), Value::text("bob@example.com"));

        ctx.load(values);

        assert_eq!(ctx.get(&test_key("name")), Some(&Value::text("Bob")));
        assert!(!ctx.is_dirty()); // Load doesn't mark dirty
    }

    #[test]
    fn test_with_values() {
        let collection = create_test_collection();

        let mut values = ParameterValues::new();
        values.set(test_key("name"), Value::text("Charlie"));

        let ctx = ParameterContext::with_values(collection, values);

        assert_eq!(ctx.get(&test_key("name")), Some(&Value::text("Charlie")));
        assert!(!ctx.is_dirty());
    }

    #[test]
    fn test_dirty_tracking() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        assert!(!ctx.is_dirty());
        assert!(ctx.dirty_keys().is_empty());

        ctx.set(test_key("name"), Value::text("Dave"));
        assert!(ctx.is_dirty());
        assert_eq!(ctx.dirty_keys(), vec![test_key("name")]);

        ctx.mark_all_clean();
        assert!(!ctx.is_dirty());
    }

    #[test]
    fn test_visibility() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        assert!(ctx.is_visible(&test_key("name")));

        ctx.set_visible(test_key("name"), false);
        assert!(!ctx.is_visible(&test_key("name")));

        ctx.set_visible(test_key("name"), true);
        assert!(ctx.is_visible(&test_key("name")));
    }

    #[test]
    fn test_clear() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        ctx.set(test_key("name"), Value::text("Eve"));
        assert!(ctx.is_dirty());

        ctx.clear();
        assert!(ctx.values().is_empty());
        assert!(!ctx.is_dirty());
    }

    #[tokio::test]
    async fn test_event_subscription() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);
        let mut rx = ctx.subscribe();

        ctx.set(test_key("name"), Value::text("Frank"));

        let event = rx.recv().await.unwrap();
        assert!(event.is_value_changed());
        assert_eq!(event.key(), Some(&test_key("name")));
    }

    #[test]
    fn test_snapshot_restore() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        ctx.set(test_key("name"), Value::text("Grace"));
        let snapshot = ctx.snapshot();

        ctx.set(test_key("name"), Value::text("Henry"));
        assert_eq!(ctx.get(&test_key("name")), Some(&Value::text("Henry")));

        ctx.restore(&snapshot);
        assert_eq!(ctx.get(&test_key("name")), Some(&Value::text("Grace")));
        assert!(!ctx.is_dirty());
    }

    #[tokio::test]
    async fn test_validate_success() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        ctx.set(test_key("name"), Value::text("Alice"));

        let result = ctx.validate(&test_key("name")).await;
        assert!(result.is_ok());
        assert!(ctx.state(&test_key("name")).unwrap().is_valid());
    }

    #[tokio::test]
    async fn test_validate_all() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        ctx.set(test_key("name"), Value::text("Bob"));
        ctx.set(test_key("email"), Value::text("bob@example.com"));

        let errors = ctx.validate_all().await;
        assert!(errors.is_empty());
        assert!(ctx.is_all_valid());
    }

    #[tokio::test]
    async fn test_set_reactive() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);
        let mut rx = ctx.subscribe();

        ctx.set_reactive(test_key("name"), Value::text("Charlie"))
            .await;

        // Should have received ValueChanged event
        let event = rx.recv().await.unwrap();
        assert!(event.is_value_changed());

        // Should have received Validated event
        let event = rx.recv().await.unwrap();
        assert!(event.is_validated());

        // Value should be set
        assert_eq!(ctx.get(&test_key("name")), Some(&Value::text("Charlie")));

        // Should be dirty and touched
        assert!(ctx.is_dirty());
        assert!(ctx.state(&test_key("name")).unwrap().is_touched());

        // Should be valid
        assert!(ctx.state(&test_key("name")).unwrap().is_valid());
    }

    #[tokio::test]
    async fn test_set_reactive_no_change() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        ctx.set(test_key("name"), Value::text("Dave"));

        let mut rx = ctx.subscribe();

        // Set same value - should not emit events
        ctx.set_reactive(test_key("name"), Value::text("Dave"))
            .await;

        // Try to receive with timeout - should fail since no events
        let result = tokio::time::timeout(std::time::Duration::from_millis(10), rx.recv()).await;

        assert!(result.is_err()); // Timeout means no events
    }

    #[test]
    fn test_all_errors() {
        let collection = create_test_collection();
        let ctx = ParameterContext::new(collection);

        // Initially all valid (no errors)
        let errors = ctx.all_errors();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_display_context() {
        let collection = create_test_collection();
        let mut ctx = ParameterContext::new(collection);

        ctx.set(test_key("name"), Value::text("Eve"));

        let display_ctx = ctx.display_context();

        // Display context should have the value
        assert_eq!(display_ctx.get("name"), Some(&Value::text("Eve")));
    }
}
