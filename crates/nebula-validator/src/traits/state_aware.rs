//! State-aware validation traits for validators with mutable state

use async_trait::async_trait;
use serde_json::Value;
use crate::types::{ValidationResult, ValidatorMetadata, ValidationComplexity};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

/// Validator that maintains state across validations
#[async_trait]
pub trait StateAwareValidator: Send + Sync {
    /// State type
    type State: Send + Sync;
    
    /// Validate with state
    async fn validate_with_state(
        &self,
        value: &Value,
        state: &mut Self::State,
    ) -> ValidationResult<()>;
    
    /// Create initial state
    fn initial_state(&self) -> Self::State;
    
    /// Reset state
    fn reset_state(&self, state: &mut Self::State);
    
    /// Get metadata
    fn metadata(&self) -> ValidatorMetadata;
}

/// Stateful validator with internal state management
#[derive(Debug)]
pub struct StatefulValidator<S> {
    state: Arc<Mutex<S>>,
    validator_fn: Arc<dyn Fn(&Value, &mut S) -> ValidationResult<()> + Send + Sync>,
    metadata: ValidatorMetadata,
}

impl<S> StatefulValidator<S>
where
    S: Default + Send + Sync + 'static,
{
    /// Create new stateful validator
    pub fn new<F>(name: impl Into<String>, validator_fn: F) -> Self
    where
        F: Fn(&Value, &mut S) -> ValidationResult<()> + Send + Sync + 'static,
    {
        Self {
            state: Arc::new(Mutex::new(S::default())),
            validator_fn: Arc::new(validator_fn),
            metadata: ValidatorMetadata::new(
                name.into(),
                "Stateful validator",
                crate::types::ValidatorCategory::Custom,
            ),
        }
    }
    
    /// Create with initial state
    pub fn with_state<F>(name: impl Into<String>, initial_state: S, validator_fn: F) -> Self
    where
        F: Fn(&Value, &mut S) -> ValidationResult<()> + Send + Sync + 'static,
    {
        Self {
            state: Arc::new(Mutex::new(initial_state)),
            validator_fn: Arc::new(validator_fn),
            metadata: ValidatorMetadata::new(
                name.into(),
                "Stateful validator",
                crate::types::ValidatorCategory::Custom,
            ),
        }
    }
    
    /// Get current state
    pub fn get_state(&self) -> S
    where
        S: Clone,
    {
        self.state.lock().unwrap().clone()
    }
    
    /// Reset state
    pub fn reset(&self)
    where
        S: Default,
    {
        *self.state.lock().unwrap() = S::default();
    }
}

#[async_trait]
impl<S> StateAwareValidator for StatefulValidator<S>
where
    S: Default + Send + Sync + 'static,
{
    type State = S;
    
    async fn validate_with_state(
        &self,
        value: &Value,
        state: &mut Self::State,
    ) -> ValidationResult<()> {
        (self.validator_fn)(value, state)
    }
    
    fn initial_state(&self) -> Self::State {
        S::default()
    }
    
    fn reset_state(&self, state: &mut Self::State) {
        *state = S::default();
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        self.metadata.clone()
    }
}

/// Counter validator that tracks validation counts
#[derive(Debug)]
pub struct CounterValidator {
    max_count: usize,
    counter: Arc<Mutex<usize>>,
    metadata: ValidatorMetadata,
}

impl CounterValidator {
    /// Create new counter validator
    pub fn new(max_count: usize) -> Self {
        Self {
            max_count,
            counter: Arc::new(Mutex::new(0)),
            metadata: ValidatorMetadata::new(
                "counter_validator",
                format!("Counter validator (max: {})", max_count),
                crate::types::ValidatorCategory::Custom,
            ),
        }
    }
    
    /// Get current count
    pub fn count(&self) -> usize {
        *self.counter.lock().unwrap()
    }
    
    /// Reset counter
    pub fn reset(&self) {
        *self.counter.lock().unwrap() = 0;
    }
}

#[async_trait]
impl crate::traits::Validatable for CounterValidator {
    async fn validate(&self, _value: &Value) -> ValidationResult<()> {
        let mut count = self.counter.lock().unwrap();
        *count += 1;
        
        if *count > self.max_count {
            ValidationResult::error(crate::types::ValidationError::new(
                crate::types::ErrorCode::RateLimitExceeded,
                format!("Validation count {} exceeds maximum {}", *count, self.max_count),
            ))
        } else {
            ValidationResult::success(())
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        self.metadata.clone()
    }
}

/// Accumulator validator that collects values
#[derive(Debug)]
pub struct AccumulatorValidator<T> {
    accumulator: Arc<Mutex<Vec<T>>>,
    max_size: Option<usize>,
    metadata: ValidatorMetadata,
}

impl<T> AccumulatorValidator<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Create new accumulator
    pub fn new(max_size: Option<usize>) -> Self {
        Self {
            accumulator: Arc::new(Mutex::new(Vec::new())),
            max_size,
            metadata: ValidatorMetadata::new(
                "accumulator_validator",
                "Accumulator validator",
                crate::types::ValidatorCategory::Custom,
            ),
        }
    }
    
    /// Get accumulated values
    pub fn get_values(&self) -> Vec<T> {
        self.accumulator.lock().unwrap().clone()
    }
    
    /// Clear accumulated values
    pub fn clear(&self) {
        self.accumulator.lock().unwrap().clear();
    }
}

/// History validator that tracks validation history
#[derive(Debug)]
pub struct HistoryValidator {
    history: Arc<Mutex<Vec<HistoryEntry>>>,
    max_history: usize,
    metadata: ValidatorMetadata,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    timestamp: chrono::DateTime<chrono::Utc>,
    value_hash: u64,
    success: bool,
}

impl HistoryValidator {
    /// Create new history validator
    pub fn new(max_history: usize) -> Self {
        Self {
            history: Arc::new(Mutex::new(Vec::new())),
            max_history,
            metadata: ValidatorMetadata::new(
                "history_validator",
                format!("History validator (max: {})", max_history),
                crate::types::ValidatorCategory::Custom,
            ),
        }
    }
    
    /// Get validation history
    pub fn get_history(&self) -> Vec<(chrono::DateTime<chrono::Utc>, bool)> {
        self.history
            .lock()
            .unwrap()
            .iter()
            .map(|entry| (entry.timestamp, entry.success))
            .collect()
    }
    
    /// Check if value was seen before
    pub fn was_seen(&self, value: &Value) -> bool {
        let hash = self.hash_value(value);
        self.history
            .lock()
            .unwrap()
            .iter()
            .any(|entry| entry.value_hash == hash)
    }
    
    fn hash_value(&self, value: &Value) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        format!("{:?}", value).hash(&mut hasher);
        hasher.finish()
    }
}

/// Session validator that maintains session state
#[derive(Debug)]
pub struct SessionValidator {
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    metadata: ValidatorMetadata,
}

#[derive(Debug, Clone)]
struct SessionState {
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: chrono::DateTime<chrono::Utc>,
    validation_count: usize,
    data: HashMap<String, Value>,
}

impl SessionValidator {
    /// Create new session validator
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            metadata: ValidatorMetadata::new(
                "session_validator",
                "Session-aware validator",
                crate::types::ValidatorCategory::Custom,
            ),
        }
    }
    
    /// Get or create session
    pub fn get_session(&self, session_id: &str) -> SessionState {
        let mut sessions = self.sessions.lock().unwrap();
        sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionState {
                created_at: chrono::Utc::now(),
                last_accessed: chrono::Utc::now(),
                validation_count: 0,
                data: HashMap::new(),
            })
            .clone()
    }
    
    /// Update session
    pub fn update_session(&self, session_id: &str, key: String, value: Value) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.last_accessed = chrono::Utc::now();
            session.validation_count += 1;
            session.data.insert(key, value);
        }
    }
    
    /// Clear old sessions
    pub fn clear_old_sessions(&self, max_age: chrono::Duration) {
        let now = chrono::Utc::now();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, session| {
            now.signed_duration_since(session.last_accessed) < max_age
        });
    }
}

impl Default for SessionValidator {
    fn default() -> Self {
        Self::new()
    }
}