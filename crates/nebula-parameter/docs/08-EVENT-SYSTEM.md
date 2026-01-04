# Event System and Reactive Patterns

Comprehensive guide to the Nebula parameter event system, observer patterns, and reactive updates.

---

## Overview

The event system enables reactive parameter behavior through:

- **ParameterEvent enum** - Type-safe event variants
- **EventBus** - Central event dispatch
- **Observer pattern** - Subscribe to parameter changes
- **DisplayObserver** - Reactive visibility updates

---

## Event Types

```rust
pub enum ParameterEvent {
    // Value lifecycle
    BeforeChange { key: String, old_value: Value, new_value: Value },
    AfterChange { key: String, old_value: Value, new_value: Value },
    
    // Validation lifecycle
    ValidationStarted { key: String },
    ValidationPassed { key: String },
    ValidationFailed { key: String, errors: Vec<ValidationError> },
    
    // User interaction
    Touched { key: String },
    Reset { key: String, value: Value },
    
    // UI state
    VisibilityChanged { key: String, visible: bool },
    EnabledChanged { key: String, enabled: bool },
    
    // Actions
    ActionTriggered { key: String, timestamp: Instant },
    
    // Batch operations
    BatchUpdate { keys: Vec<String> },
}
```

---

## EventBus Architecture

Central event dispatch with typed channels:

```rust
use tokio::sync::broadcast;

pub struct EventBus {
    sender: broadcast::Sender<ParameterEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
    
    /// Emit event to all subscribers
    pub fn emit(&self, event: ParameterEvent) {
        // Ignore error if no receivers
        let _ = self.sender.send(event);
    }
    
    /// Subscribe to all events
    pub fn subscribe(&self) -> broadcast::Receiver<ParameterEvent> {
        self.sender.subscribe()
    }
}
```

### Usage Example

```rust
let bus = EventBus::new(256);

// Spawn listener task
let mut rx = bus.subscribe();
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        match event {
            ParameterEvent::AfterChange { key, new_value, .. } => {
                println!("Parameter {} changed to {:?}", key, new_value);
            }
            ParameterEvent::ValidationFailed { key, errors } => {
                eprintln!("Validation failed for {}: {:?}", key, errors);
            }
            _ => {}
        }
    }
});

// Emit events
bus.emit(ParameterEvent::AfterChange {
    key: "username".into(),
    old_value: Value::Null,
    new_value: Value::Text("alice".into()),
});
```

---

## Observer Pattern

Fine-grained subscriptions per parameter:

```rust
use std::collections::HashMap;
use tokio::sync::mpsc;

pub struct ParameterObserver {
    bus: EventBus,
    subscriptions: HashMap<String, Vec<mpsc::Sender<ParameterEvent>>>,
}

impl ParameterObserver {
    /// Subscribe to specific parameter
    pub fn subscribe_parameter(
        &mut self,
        key: &str,
    ) -> mpsc::Receiver<ParameterEvent> {
        let (tx, rx) = mpsc::channel(32);
        self.subscriptions
            .entry(key.to_string())
            .or_default()
            .push(tx);
        rx
    }
    
    /// Subscribe with filter predicate
    pub fn subscribe_filtered<F>(
        &self,
        predicate: F,
    ) -> mpsc::Receiver<ParameterEvent>
    where
        F: Fn(&ParameterEvent) -> bool + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(32);
        let mut bus_rx = self.bus.subscribe();
        
        tokio::spawn(async move {
            while let Ok(event) = bus_rx.recv().await {
                if predicate(&event) {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            }
        });
        
        rx
    }
}
```

### Subscribe to Specific Parameter

```rust
let mut observer = ParameterObserver::new(bus);

// Only receive events for "email" parameter
let mut email_events = observer.subscribe_parameter("email");

tokio::spawn(async move {
    while let Some(event) = email_events.recv().await {
        // Handle email-specific events
    }
});
```

### Subscribe with Filter

```rust
// Only validation failures
let mut failures = observer.subscribe_filtered(|e| {
    matches!(e, ParameterEvent::ValidationFailed { .. })
});

// Only value changes
let mut changes = observer.subscribe_filtered(|e| {
    matches!(e, ParameterEvent::AfterChange { .. })
});
```

---

## Context Integration

The Context integrates with the event system:

```rust
pub struct Context {
    schema: Arc<Schema>,
    values: HashMap<String, Value>,
    states: HashMap<String, ParameterState>,
    bus: EventBus,
}

impl Context {
    pub fn set_value(&mut self, key: &str, value: Value) -> Result<()> {
        let old_value = self.values.get(key).cloned().unwrap_or(Value::Null);
        
        // Emit before change
        self.bus.emit(ParameterEvent::BeforeChange {
            key: key.into(),
            old_value: old_value.clone(),
            new_value: value.clone(),
        });
        
        // Apply transformers
        let transformed = self.apply_transformers(key, value)?;
        
        // Validate
        self.bus.emit(ParameterEvent::ValidationStarted { key: key.into() });
        
        match self.validate(key, &transformed) {
            Ok(()) => {
                self.bus.emit(ParameterEvent::ValidationPassed { key: key.into() });
            }
            Err(errors) => {
                self.bus.emit(ParameterEvent::ValidationFailed {
                    key: key.into(),
                    errors: errors.clone(),
                });
                return Err(Error::Validation(errors));
            }
        }
        
        // Store value
        self.values.insert(key.into(), transformed.clone());
        
        // Emit after change
        self.bus.emit(ParameterEvent::AfterChange {
            key: key.into(),
            old_value,
            new_value: transformed,
        });
        
        Ok(())
    }
    
    /// Subscribe to all events
    pub fn subscribe_all(&self) -> broadcast::Receiver<ParameterEvent> {
        self.bus.subscribe()
    }
    
    /// Subscribe to specific parameter
    pub fn subscribe_parameter(&self, key: &str) -> mpsc::Receiver<ParameterEvent> {
        // Implementation with filtering
        todo!()
    }
}
```

---

## DisplayObserver - Reactive Visibility

Automatically update visibility based on display rules:

```rust
pub struct DisplayObserver {
    context: Arc<RwLock<Context>>,
    display_rules: HashMap<String, DisplayRule>,
}

impl DisplayObserver {
    pub fn new(context: Arc<RwLock<Context>>) -> Self {
        Self {
            context,
            display_rules: HashMap::new(),
        }
    }
    
    /// Register display rule for parameter
    pub fn register(&mut self, key: &str, rule: DisplayRule) {
        self.display_rules.insert(key.to_string(), rule);
    }
    
    /// Start observing changes
    pub async fn start(&self) {
        let context = self.context.read().await;
        let mut events = context.subscribe_all();
        drop(context);
        
        while let Ok(event) = events.recv().await {
            if let ParameterEvent::AfterChange { key, new_value, .. } = event {
                self.evaluate_dependent_rules(&key, &new_value).await;
            }
        }
    }
    
    /// Evaluate rules that depend on changed parameter
    async fn evaluate_dependent_rules(&self, changed_key: &str, new_value: &Value) {
        let context = self.context.read().await;
        
        for (param_key, rule) in &self.display_rules {
            if rule.depends_on(changed_key) {
                let visible = rule.evaluate(&context);
                let enabled = rule.evaluate_enabled(&context);
                
                // Emit visibility/enabled changes
                if rule.visibility_changed(visible) {
                    context.bus.emit(ParameterEvent::VisibilityChanged {
                        key: param_key.clone(),
                        visible,
                    });
                }
                
                if rule.enabled_changed(enabled) {
                    context.bus.emit(ParameterEvent::EnabledChanged {
                        key: param_key.clone(),
                        enabled,
                    });
                }
            }
        }
    }
}
```

### Display Rule Definition

```rust
pub struct DisplayRule {
    pub visibility: VisibilityCondition,
    pub enabled: Option<EnabledCondition>,
    dependencies: Vec<String>,  // Tracked for reactive updates
}

pub enum VisibilityCondition {
    Always,
    Never,
    When(Condition),
    Unless(Condition),
}

pub enum Condition {
    Equals { key: String, value: Value },
    NotEquals { key: String, value: Value },
    IsEmpty { key: String },
    IsNotEmpty { key: String },
    GreaterThan { key: String, value: Value },
    LessThan { key: String, value: Value },
    And(Vec<Condition>),
    Or(Vec<Condition>),
    Not(Box<Condition>),
}

impl DisplayRule {
    /// Check if rule depends on given parameter
    pub fn depends_on(&self, key: &str) -> bool {
        self.dependencies.contains(&key.to_string())
    }
    
    /// Evaluate visibility condition
    pub fn evaluate(&self, context: &Context) -> bool {
        match &self.visibility {
            VisibilityCondition::Always => true,
            VisibilityCondition::Never => false,
            VisibilityCondition::When(cond) => cond.evaluate(context),
            VisibilityCondition::Unless(cond) => !cond.evaluate(context),
        }
    }
}
```

### Usage Example

```rust
// Parameter shows only when mode == "advanced"
let rule = DisplayRule::show_when(
    Condition::Equals {
        key: "mode".into(),
        value: Value::Text("advanced".into()),
    }
);

// Complex condition: show when (mode == "custom" AND count > 0)
let rule = DisplayRule::show_when(
    Condition::And(vec![
        Condition::Equals {
            key: "mode".into(),
            value: Value::Text("custom".into()),
        },
        Condition::GreaterThan {
            key: "count".into(),
            value: Value::Int(0),
        },
    ])
);

// Register with observer
observer.register("advanced_setting", rule);
```

---

## Event Batching

For performance, batch multiple changes:

```rust
impl Context {
    /// Start batch update (suppress individual events)
    pub fn begin_batch(&mut self) {
        self.batch_mode = true;
        self.batch_keys.clear();
    }
    
    /// End batch update (emit single BatchUpdate event)
    pub fn end_batch(&mut self) {
        if self.batch_mode {
            self.batch_mode = false;
            
            if !self.batch_keys.is_empty() {
                self.bus.emit(ParameterEvent::BatchUpdate {
                    keys: self.batch_keys.drain().collect(),
                });
            }
        }
    }
    
    /// Set value during batch (no individual event)
    fn set_value_batched(&mut self, key: &str, value: Value) -> Result<()> {
        // Transform and validate...
        self.values.insert(key.into(), value);
        self.batch_keys.insert(key.to_string());
        Ok(())
    }
}

// Usage
context.begin_batch();
context.set_value("x", Value::Float(1.0))?;
context.set_value("y", Value::Float(2.0))?;
context.set_value("z", Value::Float(3.0))?;
context.end_batch();  // Single BatchUpdate event
```

---

## Debouncing

Debounce rapid changes for expensive operations:

```rust
use tokio::time::{Duration, sleep};

pub struct DebouncedSubscriber {
    delay: Duration,
    pending: Option<ParameterEvent>,
}

impl DebouncedSubscriber {
    pub fn new(delay: Duration) -> Self {
        Self { delay, pending: None }
    }
    
    pub async fn subscribe(
        mut self,
        mut rx: broadcast::Receiver<ParameterEvent>,
        mut handler: impl FnMut(ParameterEvent),
    ) {
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(event) => {
                            self.pending = Some(event);
                        }
                        Err(_) => break,
                    }
                }
                _ = sleep(self.delay), if self.pending.is_some() => {
                    if let Some(event) = self.pending.take() {
                        handler(event);
                    }
                }
            }
        }
    }
}

// Usage: debounce validation for 300ms
let debounced = DebouncedSubscriber::new(Duration::from_millis(300));
debounced.subscribe(events, |event| {
    if let ParameterEvent::AfterChange { key, new_value, .. } = event {
        // Expensive validation or API call
        validate_with_server(&key, &new_value);
    }
}).await;
```

---

## UI Integration Pattern

Example integration with egui:

```rust
pub struct ParameterWidget {
    context: Arc<RwLock<Context>>,
    visibility_cache: HashMap<String, bool>,
    enabled_cache: HashMap<String, bool>,
}

impl ParameterWidget {
    pub fn new(context: Arc<RwLock<Context>>) -> Self {
        let mut widget = Self {
            context,
            visibility_cache: HashMap::new(),
            enabled_cache: HashMap::new(),
        };
        widget.subscribe_visibility_changes();
        widget
    }
    
    fn subscribe_visibility_changes(&mut self) {
        let context = self.context.clone();
        let visibility = Arc::new(RwLock::new(self.visibility_cache.clone()));
        let enabled = Arc::new(RwLock::new(self.enabled_cache.clone()));
        
        tokio::spawn(async move {
            let ctx = context.read().await;
            let mut events = ctx.subscribe_all();
            drop(ctx);
            
            while let Ok(event) = events.recv().await {
                match event {
                    ParameterEvent::VisibilityChanged { key, visible } => {
                        visibility.write().await.insert(key, visible);
                    }
                    ParameterEvent::EnabledChanged { key, enabled: e } => {
                        enabled.write().await.insert(key, e);
                    }
                    _ => {}
                }
            }
        });
    }
    
    pub fn render(&self, ui: &mut egui::Ui, key: &str) {
        // Check visibility
        if !self.visibility_cache.get(key).copied().unwrap_or(true) {
            return;  // Don't render hidden parameters
        }
        
        let enabled = self.enabled_cache.get(key).copied().unwrap_or(true);
        
        ui.set_enabled(enabled);
        // Render parameter widget...
    }
}
```

---

## Best Practices

### 1. Use Appropriate Channel Types

| Channel | Use Case |
|---------|----------|
| `broadcast` | Multiple subscribers, stateless events |
| `mpsc` | Single consumer, work queues |
| `oneshot` | Request-response patterns |
| `watch` | Latest value only (state) |

### 2. Handle Backpressure

```rust
// Bounded channels prevent memory issues
let (tx, rx) = mpsc::channel(256);

// Handle slow consumers
if tx.try_send(event).is_err() {
    // Log warning, drop event, or apply backpressure
    warn!("Event channel full, dropping event");
}
```

### 3. Avoid Event Loops

```rust
// BAD: Can cause infinite loop
impl Context {
    fn on_change(&mut self, key: &str) {
        // Setting value triggers change event, which calls on_change...
        self.set_value("other", compute_derived())?;
    }
}

// GOOD: Track change source
impl Context {
    fn set_value_internal(&mut self, key: &str, value: Value, emit: bool) {
        // Only emit event if not internal update
        if emit {
            self.bus.emit(ParameterEvent::AfterChange { ... });
        }
    }
}
```

### 4. Clean Up Subscriptions

```rust
// Use CancellationToken for cleanup
use tokio_util::sync::CancellationToken;

let token = CancellationToken::new();
let token_clone = token.clone();

tokio::spawn(async move {
    let mut events = bus.subscribe();
    
    loop {
        tokio::select! {
            _ = token_clone.cancelled() => {
                break;  // Clean shutdown
            }
            result = events.recv() => {
                // Handle event
            }
        }
    }
});

// Later: clean shutdown
token.cancel();
```

---

## Summary

| Component | Purpose |
|-----------|---------|
| `ParameterEvent` | Type-safe event variants |
| `EventBus` | Central broadcast dispatch |
| `ParameterObserver` | Per-parameter subscriptions |
| `DisplayObserver` | Reactive visibility updates |
| Batching | Coalesce multiple changes |
| Debouncing | Rate-limit expensive handlers |
