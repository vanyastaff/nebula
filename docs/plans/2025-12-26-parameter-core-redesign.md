# Parameter Core Redesign

**Date:** 2025-12-26  
**Status:** Approved  
**Scope:** Radical redesign of `nebula-parameter/src/core`

## Overview

Complete rewrite of the parameter core module to provide production-ready state management with strict type safety, proper event coordination, and predictable state transitions.

## Current Problems

1. **Stub implementations** — logic written as placeholders, not production-ready
2. **Inconsistent event emission** — some operations emit events, others don't
3. **Chaotic flag management** — `VALID` flag can be true with non-empty errors
4. **Weak typing** — excessive use of `String`, `Option`, raw primitives
5. **Two contracts for same operation** — `set()` vs `set_reactive()` with different behavior
6. **No state machine** — parameters can be in invalid combinations of states
7. **Mixed concerns** — interaction state and validation state in single enum

## Design Decisions

| Aspect | Decision |
|--------|----------|
| Scope | Radical redesign, production-ready |
| Structure | Flat (11 files) |
| State Model | Centralized (Context owns all state) |
| State Axes | Two orthogonal axes: Interaction + Validation |
| Events | Hierarchical (Rust idiomatic), notification-only |
| Interceptors | Sync chain for cancellable/modifiable changes |
| Async Validation | Two-phase: start_validation() + complete_validation() |
| Snapshots | Values only, availability recalculated |
| Breaking changes | Allowed (UI will also be rewritten) |

## Architecture

### File Structure

```
core/
├── mod.rs          # Public API + re-exports
├── schema.rs       # ParameterMetadata, ParameterKind, ParameterBase, capabilities
├── traits.rs       # Describable, Validatable, Displayable, Parameter
├── state.rs        # Interaction, Validation, Availability, ParameterState
├── values.rs       # ParameterValues, Snapshot, Diff
├── events.rs       # Hierarchical event types + EventBus
├── context.rs      # ParameterContext + Transaction
├── collection.rs   # ParameterCollection - schema registry
├── display.rs      # DisplayCondition, DisplayRule, DisplayContext
├── validation.rs   # ParameterValidation, ValidationHandle, two-phase flow
├── options.rs      # SelectOption, SelectOptions, dynamic loading
└── interceptor.rs  # SetInterceptor trait + implementations
```

### Two-Axis State Model

Unlike the original single-phase design, we separate **interaction** (user behavior) from **validation** (data correctness). This eliminates awkward transitions like "Valid → Editing → ?".

#### Interaction Axis

```rust
/// Tracks user interaction with the parameter
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Interaction {
    /// No user interaction yet (initial state)
    #[default]
    Pristine,
    /// User is actively editing (input focused)
    Editing,
    /// User has interacted but not currently editing (input blurred)
    Touched,
}
```

#### Validation Axis

```rust
/// Tracks validation state of the parameter value
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Validation {
    /// Not yet validated
    Unknown,
    /// Validation in progress (async)
    Pending(PendingValidation),
    /// Validation passed
    Valid,
    /// Validation failed (errors guaranteed non-empty by type)
    Invalid(NonEmptyVec<ValidationError>),
}

/// Metadata for pending async validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingValidation {
    /// When validation started
    pub started_at: Instant,
    /// Timeout for this validation
    pub timeout: Duration,
    /// Handle to track/cancel this validation
    pub handle: ValidationHandle,
}

impl PendingValidation {
    /// Check if validation has exceeded timeout
    #[inline]
    pub fn is_expired(&self) -> bool {
        self.started_at.elapsed() > self.timeout
    }
}
```

#### Availability (Orthogonal)

```rust
/// Controls whether parameter is shown/enabled in UI
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Availability {
    /// Whether parameter is visible (display conditions met)
    pub visible: bool,
    /// Whether parameter is enabled (not disabled by conditions)
    pub enabled: bool,
}

impl Default for Availability {
    fn default() -> Self {
        Self { visible: true, enabled: true }
    }
}
```

#### Combined State

```rust
/// Complete runtime state for a single parameter
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterState {
    /// User interaction state
    interaction: Interaction,
    /// Validation state
    validation: Validation,
    /// Whether value differs from last saved/loaded
    dirty: bool,
    /// Visibility and enabled state
    availability: Availability,
}
```

**Type-enforced invariants:**
- `Validation::Invalid` always contains at least one error (`NonEmptyVec`)
- `Validation::Pending` always has timeout metadata
- No invalid combinations possible at compile time

### Hierarchical Events (Notification Only)

Events are for observation, not control flow. Use interceptors for cancellation.

```rust
/// Top-level event enum (Rust idiomatic hierarchical pattern)
#[derive(Debug, Clone)]
pub enum ParameterEvent {
    /// Value-related events
    Value(ValueEvent),
    /// State transition events
    State(StateEvent),
    /// Visibility/enabled changes
    Availability(AvailabilityEvent),
    /// Lifecycle events (load, clear, snapshot, batch)
    Lifecycle(LifecycleEvent),
}

/// Value change events
#[derive(Debug, Clone)]
pub enum ValueEvent {
    /// Value has changed (notification only, not cancellable)
    Changed {
        key: ParameterKey,
        old: Value,
        new: Value,
    },
}

/// State transition events
#[derive(Debug, Clone)]
pub enum StateEvent {
    /// Interaction state changed
    InteractionChanged {
        key: ParameterKey,
        from: Interaction,
        to: Interaction,
    },
    /// Validation state changed
    ValidationChanged {
        key: ParameterKey,
        from: ValidationState,  // Simplified enum for events
        to: ValidationState,
    },
    /// Dirty flag changed
    DirtyChanged {
        key: ParameterKey,
        dirty: bool,
    },
}

/// Simplified validation state for events (no internal data)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationState {
    Unknown,
    Pending,
    Valid,
    Invalid,
}

/// Visibility/enabled changes
#[derive(Debug, Clone)]
pub enum AvailabilityEvent {
    /// Visibility changed
    VisibilityChanged {
        key: ParameterKey,
        visible: bool,
    },
    /// Enabled state changed
    EnabledChanged {
        key: ParameterKey,
        enabled: bool,
    },
}

/// Lifecycle events
#[derive(Debug, Clone)]
pub enum LifecycleEvent {
    /// Values loaded (initial or reset)
    Loaded {
        keys: Vec<ParameterKey>,
    },
    /// All values cleared
    Cleared,
    /// Snapshot created
    SnapshotCreated {
        id: SnapshotId,
        tag: Option<String>,
    },
    /// Restored from snapshot
    Restored {
        id: SnapshotId,
    },
    /// Batch change completed
    BatchChanged {
        keys: Vec<ParameterKey>,
    },
}
```

### Interceptor Chain (For Cancellation/Modification)

Interceptors provide synchronous hooks for business logic before changes apply.

```rust
/// Result of intercepting a set operation
#[derive(Debug, Clone)]
pub enum InterceptResult {
    /// Allow the change to proceed
    Allow,
    /// Reject the change with a reason
    Deny { reason: String },
    /// Allow but modify the value
    Modify { value: Value },
}

/// Trait for intercepting value changes
pub trait SetInterceptor: Send + Sync {
    /// Called before a value change is applied
    /// 
    /// Interceptors run synchronously in order. First `Deny` stops the chain.
    fn intercept(
        &self,
        key: &ParameterKey,
        current: Option<&Value>,
        proposed: &Value,
    ) -> InterceptResult;
}

/// Context provides interceptor management
impl ParameterContext {
    /// Add an interceptor to the chain
    pub fn add_interceptor(&mut self, interceptor: Box<dyn SetInterceptor>);
    
    /// Remove all interceptors
    pub fn clear_interceptors(&mut self);
}
```

**Use cases:**
- Enforce business rules ("price cannot be negative")
- Coerce values ("round to 2 decimal places")
- Prevent destructive changes (confirmation required)

### Two-Phase Async Validation

Async validation without holding `&mut self` across await points.

```rust
/// Unique identifier for a validation operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValidationHandle(u64);

/// Result of validation
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// Validation passed
    Passed,
    /// Validation failed with errors
    Failed(NonEmptyVec<ValidationError>),
    /// Validation was cancelled (superseded or explicit)
    Cancelled,
    /// Validation timed out
    TimedOut,
}

impl ParameterContext {
    /// Phase 1: Start async validation (sync, returns immediately)
    /// 
    /// - Transitions validation state to Pending
    /// - Returns handle for tracking
    /// - Emits StateEvent::ValidationChanged
    pub fn start_validation(
        &mut self, 
        key: &ParameterKey,
        timeout: Duration,
    ) -> Result<ValidationHandle, ValidationError> {
        // 1. Get current value (clone to release borrow)
        // 2. Set state.validation = Validation::Pending(...)
        // 3. Emit event
        // 4. Return handle
    }
    
    /// Phase 2: Complete validation (sync, called after async work)
    /// 
    /// - Checks handle is still valid (not cancelled/superseded)
    /// - Updates validation state to Valid/Invalid
    /// - Emits StateEvent::ValidationChanged
    pub fn complete_validation(
        &mut self,
        handle: ValidationHandle,
        result: ValidationResult,
    ) -> Result<(), ValidationError> {
        // 1. Verify handle matches current pending validation
        // 2. Update state based on result
        // 3. Emit event
    }
    
    /// Cancel a pending validation
    pub fn cancel_validation(&mut self, handle: ValidationHandle);
    
    /// Check for timed-out validations and mark them
    pub fn check_validation_timeouts(&mut self) -> Vec<ParameterKey>;
}
```

**Usage pattern:**
```rust
// Start validation (sync)
let handle = context.start_validation(&key, Duration::from_secs(5))?;
let value = context.get(&key).cloned();
let schema = context.collection().get_validatable(&key);

// Spawn async work (no borrow held)
let result_tx = result_tx.clone();
tokio::spawn(async move {
    let result = match schema.validate(&value).await {
        Ok(()) => ValidationResult::Passed,
        Err(errors) => ValidationResult::Failed(errors),
    };
    let _ = result_tx.send((handle, result)).await;
});

// Later, in event loop:
while let Some((handle, result)) = result_rx.recv().await {
    context.complete_validation(handle, result)?;
}
```

### Centralized Context

```rust
pub struct ParameterContext {
    /// Parameter schemas (immutable after construction)
    collection: Arc<ParameterCollection>,
    
    /// Current values
    values: ParameterValues,
    
    /// State for each parameter
    states: HashMap<ParameterKey, ParameterState>,
    
    /// Set interceptors (sync chain)
    interceptors: Vec<Box<dyn SetInterceptor>>,
    
    /// Event broadcaster
    event_bus: EventBus,
    
    /// Snapshot history for undo/redo
    snapshots: SnapshotHistory,
    
    /// Next validation handle ID
    next_validation_id: u64,
}

impl ParameterContext {
    // ===================
    // Value Operations
    // ===================
    
    /// Set a value (THE way to change values)
    /// 
    /// 1. Run interceptor chain (may reject/modify)
    /// 2. Update value
    /// 3. Set interaction = Editing (if Pristine) or keep current
    /// 4. Set dirty = true
    /// 5. Set validation = Unknown (value changed, needs revalidation)
    /// 6. Emit ValueEvent::Changed
    /// 7. Update dependent availability
    pub fn set(&mut self, key: ParameterKey, value: Value) -> Result<(), SetError>;
    
    /// Set multiple values atomically (single event)
    pub fn set_batch(
        &mut self,
        changes: impl IntoIterator<Item = (ParameterKey, Value)>,
    ) -> Result<BatchResult, BatchError>;
    
    /// Get a value
    pub fn get(&self, key: &ParameterKey) -> Option<&Value>;
    
    /// Get typed value (hot path, no allocation)
    pub fn get_as<T: TryFromValue>(&self, key: &ParameterKey) -> Result<T, ValueAccessError>;
    
    // ===================
    // State Operations
    // ===================
    
    /// Mark parameter as being edited (focus gained)
    pub fn begin_editing(&mut self, key: &ParameterKey);
    
    /// Mark parameter as no longer being edited (focus lost)
    pub fn end_editing(&mut self, key: &ParameterKey);
    
    /// Get current state
    pub fn state(&self, key: &ParameterKey) -> Option<&ParameterState>;
    
    // ===================
    // Validation
    // ===================
    
    /// Start async validation (phase 1)
    pub fn start_validation(&mut self, key: &ParameterKey, timeout: Duration) -> Result<ValidationHandle, ValidationError>;
    
    /// Complete async validation (phase 2)
    pub fn complete_validation(&mut self, handle: ValidationHandle, result: ValidationResult) -> Result<(), ValidationError>;
    
    /// Validate all parameters, returns summary
    pub async fn validate_all(&mut self) -> ValidationSummary;
    
    // ===================
    // Snapshots & Transactions
    // ===================
    
    /// Create a snapshot of current values
    pub fn create_snapshot(&mut self, tag: Option<String>) -> SnapshotId;
    
    /// Restore from a snapshot
    pub fn restore_snapshot(&mut self, id: SnapshotId) -> Result<(), SnapshotError>;
    
    /// Begin a transaction (auto-creates snapshot)
    pub fn begin_transaction(&mut self) -> Transaction;
    
    /// Commit a transaction
    pub fn commit(&mut self, tx: Transaction);
    
    /// Rollback a transaction
    pub fn rollback(&mut self, tx: Transaction) -> Result<(), SnapshotError>;
    
    // ===================
    // Events
    // ===================
    
    /// Subscribe to events
    pub fn subscribe(&self) -> EventSubscription;
    
    /// Get current snapshot (source of truth for resync after missed events)
    pub fn snapshot(&self) -> ContextSnapshot;
    
    // ===================
    // Debug
    // ===================
    
    /// Get diagnostic info
    pub fn debug_info(&self) -> ContextDebugInfo;
}
```

### Strongly Typed Value Access

Two-layer API: hot path (no allocation) and convenience (allocates).

```rust
impl ParameterValues {
    /// Hot path: get typed value by reference (no allocation)
    pub fn get<T: TryFromValue>(&self, key: &ParameterKey) -> Result<T, ValueAccessError>;
    
    /// Hot path: get or default
    pub fn get_or<T: TryFromValue>(&self, key: &ParameterKey, default: T) -> T;
    
    /// Hot path: get optional (None if missing, Some if present)
    pub fn get_opt<T: TryFromValue>(&self, key: &ParameterKey) -> Result<Option<T>, ValueAccessError>;
    
    /// Convenience: get by string name (allocates ParameterKey)
    pub fn get_by_name<T: TryFromValue>(&self, name: &str) -> Result<T, ValueAccessError>;
}
```

### Snapshot System

Snapshots store only values. State is recalculated on restore.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotId(u64);

#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Unique identifier
    pub id: SnapshotId,
    /// Stored values
    pub values: HashMap<ParameterKey, Value>,
    /// When snapshot was created
    pub timestamp: Instant,
    /// Optional human-readable tag
    pub tag: Option<String>,
}

pub struct SnapshotHistory {
    snapshots: VecDeque<Snapshot>,
    max_size: usize,
    next_id: u64,
}

impl SnapshotHistory {
    /// Create a snapshot
    pub fn create(&mut self, values: &ParameterValues, tag: Option<String>) -> SnapshotId;
    
    /// Get a snapshot by ID
    pub fn get(&self, id: SnapshotId) -> Option<&Snapshot>;
    
    /// List all snapshots (newest first)
    pub fn list(&self) -> impl Iterator<Item = &Snapshot>;
    
    /// Remove old snapshots beyond max_size
    fn prune(&mut self);
}
```

**On restore:**
```rust
impl ParameterContext {
    pub fn restore_snapshot(&mut self, id: SnapshotId) -> Result<(), SnapshotError> {
        let snapshot = self.snapshots.get(id)?;
        
        // 1. Restore values
        self.values = ParameterValues::from(snapshot.values.clone());
        
        // 2. Reset all states to pristine
        for state in self.states.values_mut() {
            state.interaction = Interaction::Pristine;
            state.validation = Validation::Unknown;
            state.dirty = false;
            // availability NOT reset - will be recalculated
        }
        
        // 3. Recalculate availability from display conditions
        self.recalculate_all_availability();
        
        // 4. Emit event
        self.event_bus.emit(ParameterEvent::Lifecycle(
            LifecycleEvent::Restored { id }
        ));
        
        Ok(())
    }
}
```

### Transaction Support

For complex operations that need atomicity.

```rust
pub struct Transaction {
    snapshot_id: SnapshotId,
    committed: bool,
}

impl ParameterContext {
    pub fn begin_transaction(&mut self) -> Transaction {
        Transaction {
            snapshot_id: self.create_snapshot(Some("transaction".into())),
            committed: false,
        }
    }
    
    pub fn commit(&mut self, mut tx: Transaction) {
        tx.committed = true;
        // Snapshot remains in history
    }
    
    pub fn rollback(&mut self, tx: Transaction) -> Result<(), SnapshotError> {
        if !tx.committed {
            self.restore_snapshot(tx.snapshot_id)?;
        }
        Ok(())
    }
}
```

### Event Bus

```rust
pub struct EventBus {
    sender: broadcast::Sender<ParameterEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self;
    
    /// Emit an event (fire-and-forget)
    pub fn emit(&self, event: ParameterEvent);
    
    /// Subscribe to events
    pub fn subscribe(&self) -> EventSubscription;
}

pub struct EventSubscription {
    receiver: broadcast::Receiver<ParameterEvent>,
}

impl EventSubscription {
    /// Receive next event
    pub async fn recv(&mut self) -> Result<ParameterEvent, RecvError>;
    
    /// Try receive without blocking
    pub fn try_recv(&mut self) -> Result<ParameterEvent, TryRecvError>;
}
```

**Contract:**
- Events are ephemeral notifications (may be missed due to lag)
- Source of truth is always `context.snapshot()` or direct queries
- Use events for reactive updates, resync from context when needed

### Debug Utilities

```rust
#[derive(Debug, Clone)]
pub struct ContextDebugInfo {
    /// Total number of parameters
    pub total_params: usize,
    /// Count by interaction state
    pub by_interaction: HashMap<Interaction, usize>,
    /// Count by validation state
    pub by_validation: HashMap<ValidationState, usize>,
    /// Number of dirty parameters
    pub dirty_count: usize,
    /// Number of pending validations
    pub pending_validations: usize,
    /// Total error count across all parameters
    pub total_errors: usize,
    /// Parameters with errors
    pub error_keys: Vec<ParameterKey>,
}

impl ParameterContext {
    pub fn debug_info(&self) -> ContextDebugInfo;
}
```

## NonEmptyVec Utility

To enforce "Invalid always has errors" at type level:

```rust
/// A Vec that is guaranteed to have at least one element
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyVec<T> {
    first: T,
    rest: Vec<T>,
}

impl<T> NonEmptyVec<T> {
    /// Create with a single element
    pub fn new(first: T) -> Self {
        Self { first, rest: Vec::new() }
    }
    
    /// Create from a non-empty vec (returns None if empty)
    pub fn from_vec(mut vec: Vec<T>) -> Option<Self> {
        if vec.is_empty() {
            None
        } else {
            let first = vec.remove(0);
            Some(Self { first, rest: vec })
        }
    }
    
    /// Get first element
    pub fn first(&self) -> &T {
        &self.first
    }
    
    /// Get all elements as slice
    pub fn as_slice(&self) -> impl Iterator<Item = &T> {
        std::iter::once(&self.first).chain(self.rest.iter())
    }
    
    /// Get length (always >= 1)
    pub fn len(&self) -> usize {
        1 + self.rest.len()
    }
    
    /// Push an element
    pub fn push(&mut self, item: T) {
        self.rest.push(item);
    }
}
```

## Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum SetError {
    #[error("Parameter '{key}' not found")]
    NotFound { key: ParameterKey },
    
    #[error("Change to '{key}' was rejected: {reason}")]
    Rejected { key: ParameterKey, reason: String },
    
    #[error("Cannot modify '{key}': parameter is disabled")]
    Disabled { key: ParameterKey },
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Parameter '{key}' not found")]
    NotFound { key: ParameterKey },
    
    #[error("Validation already pending for '{key}'")]
    AlreadyPending { key: ParameterKey },
    
    #[error("Invalid validation handle")]
    InvalidHandle,
    
    #[error("Validation timed out for '{key}'")]
    TimedOut { key: ParameterKey },
}

#[derive(Debug, thiserror::Error)]
pub enum ValueAccessError {
    #[error("Parameter '{0}' not found")]
    NotFound(ParameterKey),
    
    #[error("Type mismatch for '{key}': expected {expected}, got {actual}")]
    TypeMismatch {
        key: ParameterKey,
        expected: &'static str,
        actual: ValueKind,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("Snapshot {0:?} not found")]
    NotFound(SnapshotId),
}

#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("Batch operation failed for '{key}': {source}")]
    SetFailed {
        key: ParameterKey,
        #[source]
        source: SetError,
    },
}
```

## Implementation Priority

1. **Core types** (`state.rs`) — Interaction, Validation, Availability, ParameterState, NonEmptyVec
2. **Events** (`events.rs`) — hierarchical events + EventBus
3. **Values** (`values.rs`) — ParameterValues, Snapshot, Diff
4. **Schema** (`schema.rs`) — ParameterMetadata, ParameterKind, ParameterBase
5. **Traits** (`traits.rs`) — Describable, Validatable, Displayable
6. **Collection** (`collection.rs`) — ParameterCollection
7. **Context basic** (`context.rs`) — set/get, state coordination
8. **Interceptors** (`interceptor.rs`) — SetInterceptor chain
9. **Validation** (`validation.rs`) — two-phase async validation flow
10. **Display** (`display.rs`) — conditions system
11. **Batch + Transactions** — advanced features

## Testing Strategy

1. **State machine tests** — verify all valid transitions work, invalid transitions rejected
2. **Invariant tests** — `Validation::Invalid` always has errors, etc.
3. **Event emission tests** — every operation emits correct events in correct order
4. **Interceptor tests** — Allow/Deny/Modify all work correctly
5. **Async validation tests** — two-phase flow, timeout handling, cancellation
6. **Snapshot tests** — create/restore, state recalculation
7. **Transaction tests** — commit/rollback semantics
8. **Integration tests** — full workflows (load → edit → validate → save → undo)
9. **Concurrency tests** — multiple subscribers, rapid updates, event lag handling

## Success Criteria

- [ ] Two-axis state model implemented (Interaction + Validation)
- [ ] Type-enforced invariants (NonEmptyVec for Invalid)
- [ ] All state transitions explicit and type-safe
- [ ] Interceptor chain for cancellable changes
- [ ] Two-phase async validation without &mut borrow across await
- [ ] Events are notification-only (no control flow)
- [ ] Snapshots store values only, state recalculated
- [ ] Transaction support for atomic operations
- [ ] Full test coverage
- [ ] `cargo clippy` passes with no warnings
- [ ] Documentation for all public types
