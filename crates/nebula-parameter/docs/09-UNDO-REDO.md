# Undo/Redo System

Command pattern implementation for reversible parameter operations.

---

## Overview

The undo/redo system provides:

- **Command trait** - Encapsulates reversible operations
- **HistoryManager** - Manages undo/redo stacks
- **MacroCommand** - Batch multiple commands as transactions
- **Snapshot/Restore** - Full state capture for complex undo

---

## Command Pattern

### Command Trait

```rust
pub trait Command: Send + Sync {
    /// Execute the command
    fn execute(&self, context: &mut Context) -> Result<()>;
    
    /// Reverse the command
    fn undo(&self, context: &mut Context) -> Result<()>;
    
    /// Human-readable description
    fn description(&self) -> &str;
    
    /// Can this command be merged with another?
    fn merge(&self, other: &dyn Command) -> Option<Box<dyn Command>> {
        None  // Default: no merging
    }
}
```

### SetValueCommand

```rust
pub struct SetValueCommand {
    key: String,
    old_value: Value,
    new_value: Value,
    description: String,
}

impl SetValueCommand {
    pub fn new(key: impl Into<String>, old_value: Value, new_value: Value) -> Self {
        let key = key.into();
        Self {
            description: format!("Set {} to {:?}", key, new_value),
            key,
            old_value,
            new_value,
        }
    }
}

impl Command for SetValueCommand {
    fn execute(&self, context: &mut Context) -> Result<()> {
        context.set_value_without_history(&self.key, self.new_value.clone())
    }
    
    fn undo(&self, context: &mut Context) -> Result<()> {
        context.set_value_without_history(&self.key, self.old_value.clone())
    }
    
    fn description(&self) -> &str {
        &self.description
    }
    
    /// Merge consecutive changes to same key
    fn merge(&self, other: &dyn Command) -> Option<Box<dyn Command>> {
        let other = other.as_any().downcast_ref::<SetValueCommand>()?;
        
        if self.key == other.key {
            Some(Box::new(SetValueCommand {
                key: self.key.clone(),
                old_value: self.old_value.clone(),
                new_value: other.new_value.clone(),
                description: format!("Set {} to {:?}", self.key, other.new_value),
            }))
        } else {
            None
        }
    }
}
```

### ResetCommand

```rust
pub struct ResetCommand {
    key: String,
    old_value: Value,
    default_value: Value,
}

impl ResetCommand {
    pub fn new(key: impl Into<String>, context: &Context) -> Result<Self> {
        let key = key.into();
        let old_value = context.get_value(&key)?;
        let default_value = context.schema.get_parameter(&key)?.default_value()
            .ok_or(Error::NoDefaultValue)?;
        
        Ok(Self { key, old_value, default_value })
    }
}

impl Command for ResetCommand {
    fn execute(&self, context: &mut Context) -> Result<()> {
        context.set_value_without_history(&self.key, self.default_value.clone())
    }
    
    fn undo(&self, context: &mut Context) -> Result<()> {
        context.set_value_without_history(&self.key, self.old_value.clone())
    }
    
    fn description(&self) -> &str {
        "Reset to default"
    }
}
```

---

## HistoryManager

Manages undo and redo stacks:

```rust
pub struct HistoryManager {
    undo_stack: Vec<Box<dyn Command>>,
    redo_stack: Vec<Box<dyn Command>>,
    max_history: usize,
    merge_timeout: Duration,
    last_command_time: Option<Instant>,
}

impl HistoryManager {
    pub fn new(max_history: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history,
            merge_timeout: Duration::from_millis(500),
            last_command_time: None,
        }
    }
    
    /// Execute command and add to history
    pub fn execute(&mut self, command: Box<dyn Command>, context: &mut Context) -> Result<()> {
        command.execute(context)?;
        
        // Try to merge with previous command
        let should_merge = self.last_command_time
            .map(|t| t.elapsed() < self.merge_timeout)
            .unwrap_or(false);
        
        if should_merge {
            if let Some(prev) = self.undo_stack.last() {
                if let Some(merged) = prev.merge(command.as_ref()) {
                    self.undo_stack.pop();
                    self.undo_stack.push(merged);
                    self.redo_stack.clear();
                    self.last_command_time = Some(Instant::now());
                    return Ok(());
                }
            }
        }
        
        // Add to undo stack
        self.undo_stack.push(command);
        
        // Clear redo stack (new action invalidates redo history)
        self.redo_stack.clear();
        
        // Limit history size
        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
        
        self.last_command_time = Some(Instant::now());
        Ok(())
    }
    
    /// Undo last command
    pub fn undo(&mut self, context: &mut Context) -> Result<bool> {
        if let Some(command) = self.undo_stack.pop() {
            command.undo(context)?;
            self.redo_stack.push(command);
            Ok(true)
        } else {
            Ok(false)  // Nothing to undo
        }
    }
    
    /// Redo last undone command
    pub fn redo(&mut self, context: &mut Context) -> Result<bool> {
        if let Some(command) = self.redo_stack.pop() {
            command.execute(context)?;
            self.undo_stack.push(command);
            Ok(true)
        } else {
            Ok(false)  // Nothing to redo
        }
    }
    
    /// Check if undo is available
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }
    
    /// Check if redo is available
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
    
    /// Get description of next undo action
    pub fn undo_description(&self) -> Option<&str> {
        self.undo_stack.last().map(|c| c.description())
    }
    
    /// Get description of next redo action
    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack.last().map(|c| c.description())
    }
    
    /// Clear all history
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}
```

---

## MacroCommand (Transactions)

Batch multiple commands as a single undoable unit:

```rust
pub struct MacroCommand {
    commands: Vec<Box<dyn Command>>,
    description: String,
}

impl MacroCommand {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            commands: Vec::new(),
            description: description.into(),
        }
    }
    
    pub fn add(&mut self, command: Box<dyn Command>) {
        self.commands.push(command);
    }
    
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

impl Command for MacroCommand {
    fn execute(&self, context: &mut Context) -> Result<()> {
        for command in &self.commands {
            command.execute(context)?;
        }
        Ok(())
    }
    
    fn undo(&self, context: &mut Context) -> Result<()> {
        // Undo in reverse order
        for command in self.commands.iter().rev() {
            command.undo(context)?;
        }
        Ok(())
    }
    
    fn description(&self) -> &str {
        &self.description
    }
}
```

### Transaction Builder

```rust
pub struct Transaction<'a> {
    history: &'a mut HistoryManager,
    context: &'a mut Context,
    commands: MacroCommand,
    committed: bool,
}

impl<'a> Transaction<'a> {
    pub fn begin(
        history: &'a mut HistoryManager,
        context: &'a mut Context,
        description: impl Into<String>,
    ) -> Self {
        Self {
            history,
            context,
            commands: MacroCommand::new(description),
            committed: false,
        }
    }
    
    /// Add a set value operation to the transaction
    pub fn set_value(&mut self, key: &str, value: Value) -> Result<()> {
        let old_value = self.context.get_value(key)?;
        let command = SetValueCommand::new(key, old_value, value.clone());
        
        // Execute immediately
        self.context.set_value_without_history(key, value)?;
        
        // Record for undo
        self.commands.add(Box::new(command));
        
        Ok(())
    }
    
    /// Commit the transaction
    pub fn commit(mut self) -> Result<()> {
        if !self.commands.is_empty() {
            // Add macro command to history (already executed)
            self.history.undo_stack.push(Box::new(self.commands));
            self.history.redo_stack.clear();
        }
        self.committed = true;
        Ok(())
    }
    
    /// Rollback the transaction
    pub fn rollback(self) -> Result<()> {
        // Undo all commands in reverse order
        self.commands.undo(self.context)?;
        Ok(())
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.committed {
            // Auto-rollback on drop if not committed
            let _ = self.commands.undo(self.context);
        }
    }
}
```

### Usage Example

```rust
// Single command
let old = context.get_value("name")?;
let command = SetValueCommand::new("name", old, Value::Text("Alice".into()));
history.execute(Box::new(command), &mut context)?;

// Transaction (multiple commands as one undo unit)
let mut tx = Transaction::begin(&mut history, &mut context, "Update position");
tx.set_value("x", Value::Float(10.0))?;
tx.set_value("y", Value::Float(20.0))?;
tx.set_value("z", Value::Float(30.0))?;
tx.commit()?;

// Undo entire transaction at once
history.undo(&mut context)?;  // Reverts x, y, z together
```

---

## Snapshot/Restore

For complex undo scenarios, capture full state:

```rust
pub struct Snapshot {
    values: HashMap<String, Value>,
    states: HashMap<String, ParameterState>,
    timestamp: Instant,
    description: String,
}

impl Snapshot {
    pub fn capture(context: &Context, description: impl Into<String>) -> Self {
        Self {
            values: context.values.clone(),
            states: context.states.clone(),
            timestamp: Instant::now(),
            description: description.into(),
        }
    }
}

impl Context {
    /// Take snapshot of current state
    pub fn snapshot(&self, description: impl Into<String>) -> Snapshot {
        Snapshot::capture(self, description)
    }
    
    /// Restore from snapshot
    pub fn restore(&mut self, snapshot: &Snapshot) {
        self.values = snapshot.values.clone();
        self.states = snapshot.states.clone();
        
        // Emit batch update event
        self.bus.emit(ParameterEvent::BatchUpdate {
            keys: self.values.keys().cloned().collect(),
        });
    }
}
```

### SnapshotCommand

```rust
pub struct SnapshotCommand {
    before: Snapshot,
    after: Snapshot,
}

impl SnapshotCommand {
    pub fn new(before: Snapshot, after: Snapshot) -> Self {
        Self { before, after }
    }
}

impl Command for SnapshotCommand {
    fn execute(&self, context: &mut Context) -> Result<()> {
        context.restore(&self.after);
        Ok(())
    }
    
    fn undo(&self, context: &mut Context) -> Result<()> {
        context.restore(&self.before);
        Ok(())
    }
    
    fn description(&self) -> &str {
        &self.after.description
    }
}
```

### Usage Example

```rust
// Capture before state
let before = context.snapshot("Before import");

// Perform complex operation
import_configuration(&mut context, config)?;

// Capture after state
let after = context.snapshot("Import configuration");

// Add to history
let command = SnapshotCommand::new(before, after);
history.undo_stack.push(Box::new(command));
```

---

## Context with History

Integrated context with automatic history tracking:

```rust
pub struct HistoryContext {
    context: Context,
    history: HistoryManager,
}

impl HistoryContext {
    pub fn new(schema: Arc<Schema>, max_history: usize) -> Self {
        Self {
            context: Context::new(schema),
            history: HistoryManager::new(max_history),
        }
    }
    
    /// Set value with history tracking
    pub fn set_value(&mut self, key: &str, value: Value) -> Result<()> {
        let old_value = self.context.get_value(key)?;
        
        if old_value != value {
            let command = SetValueCommand::new(key, old_value, value);
            self.history.execute(Box::new(command), &mut self.context)?;
        }
        
        Ok(())
    }
    
    /// Set value without recording to history
    pub fn set_value_silent(&mut self, key: &str, value: Value) -> Result<()> {
        self.context.set_value_without_history(key, value)
    }
    
    /// Start a transaction
    pub fn begin_transaction(&mut self, description: impl Into<String>) -> Transaction<'_> {
        Transaction::begin(&mut self.history, &mut self.context, description)
    }
    
    /// Undo last action
    pub fn undo(&mut self) -> Result<bool> {
        self.history.undo(&mut self.context)
    }
    
    /// Redo last undone action
    pub fn redo(&mut self) -> Result<bool> {
        self.history.redo(&mut self.context)
    }
    
    /// Reset parameter to default (with history)
    pub fn reset(&mut self, key: &str) -> Result<()> {
        let command = ResetCommand::new(key, &self.context)?;
        self.history.execute(Box::new(command), &mut self.context)
    }
    
    /// Reset all parameters (with history)
    pub fn reset_all(&mut self) -> Result<()> {
        let before = self.context.snapshot("Before reset all");
        self.context.reset_all()?;
        let after = self.context.snapshot("Reset all parameters");
        
        let command = SnapshotCommand::new(before, after);
        self.history.undo_stack.push(Box::new(command));
        self.history.redo_stack.clear();
        
        Ok(())
    }
}
```

---

## UI Integration

Example integration with keyboard shortcuts:

```rust
impl HistoryContext {
    /// Handle keyboard shortcut
    pub fn handle_key(&mut self, key: KeyCode, modifiers: Modifiers) -> bool {
        if modifiers.ctrl() {
            match key {
                KeyCode::Z if modifiers.shift() => {
                    // Ctrl+Shift+Z = Redo
                    self.redo().unwrap_or(false)
                }
                KeyCode::Z => {
                    // Ctrl+Z = Undo
                    self.undo().unwrap_or(false)
                }
                KeyCode::Y => {
                    // Ctrl+Y = Redo (Windows style)
                    self.redo().unwrap_or(false)
                }
                _ => false,
            }
        } else {
            false
        }
    }
}

// UI menu items
fn render_edit_menu(ui: &mut egui::Ui, context: &mut HistoryContext) {
    let undo_label = context.history.undo_description()
        .map(|d| format!("Undo: {}", d))
        .unwrap_or_else(|| "Undo".to_string());
    
    if ui.add_enabled(context.history.can_undo(), egui::Button::new(&undo_label)).clicked() {
        context.undo().ok();
    }
    
    let redo_label = context.history.redo_description()
        .map(|d| format!("Redo: {}", d))
        .unwrap_or_else(|| "Redo".to_string());
    
    if ui.add_enabled(context.history.can_redo(), egui::Button::new(&redo_label)).clicked() {
        context.redo().ok();
    }
}
```

---

## Best Practices

### 1. Use Transactions for Related Changes

```rust
// BAD: Each change is a separate undo step
context.set_value("x", x)?;
context.set_value("y", y)?;
context.set_value("z", z)?;

// GOOD: All changes are one undo step
let mut tx = context.begin_transaction("Move object");
tx.set_value("x", x)?;
tx.set_value("y", y)?;
tx.set_value("z", z)?;
tx.commit()?;
```

### 2. Enable Command Merging for Rapid Changes

```rust
// Typing "Hello" creates 5 commands without merging
// With merging: single command "Set text to 'Hello'"

impl SetValueCommand {
    fn merge(&self, other: &dyn Command) -> Option<Box<dyn Command>> {
        let other = other.as_any().downcast_ref::<SetValueCommand>()?;
        
        // Only merge if same key and within timeout
        if self.key == other.key {
            Some(Box::new(SetValueCommand {
                key: self.key.clone(),
                old_value: self.old_value.clone(),  // Keep original
                new_value: other.new_value.clone(), // Take latest
                description: format!("Set {}", self.key),
            }))
        } else {
            None
        }
    }
}
```

### 3. Use Snapshots for Complex Operations

```rust
// For import, paste, or other bulk operations
let before = context.snapshot("Before paste");

paste_from_clipboard(&mut context)?;

let after = context.snapshot("Paste configuration");
history.add(SnapshotCommand::new(before, after));
```

### 4. Clear History on Save

```rust
fn save_document(context: &mut HistoryContext) -> Result<()> {
    // Serialize and save...
    save_to_file(context)?;
    
    // Clear history (saved state is now baseline)
    context.history.clear();
    
    Ok(())
}
```

---

## ParameterValues API (Low-Level)

The `ParameterValues` struct provides low-level snapshot and diff functionality that powers the undo/redo system:

### Snapshot and Restore

```rust
use nebula_parameter::ParameterValues;

let mut values = ParameterValues::new();
values.set(key("name"), Value::text("Alice"));
values.set(key("age"), Value::integer(30));

// Take a snapshot
let snapshot = values.snapshot();

// Make changes
values.set(key("name"), Value::text("Bob"));
values.set(key("age"), Value::integer(25));

// Full restore (replaces all values)
values.restore(&snapshot);
assert_eq!(values.get_string(key("name"))?, "Alice");

// Partial restore (only overwrites snapshot keys, keeps others)
values.set(key("email"), Value::text("bob@example.com"));
values.restore_partial(&snapshot);
// name and age restored, email unchanged
```

### Diff and Change Tracking

```rust
let mut old_values = ParameterValues::new();
old_values.set(key("a"), Value::integer(1));
old_values.set(key("b"), Value::integer(2));

let mut new_values = ParameterValues::new();
new_values.set(key("a"), Value::integer(10));  // Changed
new_values.set(key("c"), Value::integer(3));   // Added
// b is removed

// Create diff
let diff = old_values.diff(&new_values);

// Inspect changes
assert_eq!(diff.added.len(), 1);    // c added
assert_eq!(diff.removed.len(), 1);  // b removed
assert_eq!(diff.changed.len(), 1);  // a changed: (1, 10)

// Apply diff (forward)
let mut values = old_values.clone();
diff.apply(&mut values);
assert_eq!(values, new_values);

// Reverse diff (for undo)
let undo_diff = diff.reverse();
undo_diff.apply(&mut values);
assert_eq!(values, old_values);
```

### Batch Operations with Validation

```rust
// Set multiple values with validation
let result = values.try_set_many(
    [
        (key("port"), Value::integer(8080)),
        (key("host"), Value::text("localhost")),
        (key("timeout"), Value::integer(-1)),  // Invalid!
    ],
    |key, value| {
        // Validate each value
        if key.as_str() == "timeout" {
            if let Some(n) = value.as_integer() {
                if n < 0 {
                    return Err(Error::validation("timeout must be >= 0"));
                }
            }
        }
        Ok(())
    },
);

// Returns Err with all validation failures
match result {
    Ok(()) => println!("All values set"),
    Err(errors) => {
        for (key, error) in errors {
            println!("Validation failed for {}: {}", key, error);
        }
    }
}
```

### Merge Operations

```rust
let mut base = ParameterValues::new();
base.set(key("a"), Value::integer(1));
base.set(key("b"), Value::integer(2));

let mut overlay = ParameterValues::new();
overlay.set(key("b"), Value::integer(20));  // Override
overlay.set(key("c"), Value::integer(3));   // New

// Simple merge (overlay wins)
base.merge(overlay);
// Result: a=1, b=20, c=3

// Merge with custom function
base.merge_with(overlay, |key, existing, incoming| {
    // Custom merge logic per key
    if key.as_str() == "count" {
        // Sum counts
        let e = existing.as_integer().unwrap_or(0);
        let i = incoming.as_integer().unwrap_or(0);
        Value::integer(e + i)
    } else {
        incoming  // Default: take incoming
    }
});
```

### Validation Helpers

```rust
// Check required fields
let required = vec![key("name"), key("email"), key("password")];
match values.has_all_required(&required) {
    Ok(()) => println!("All required fields present"),
    Err(missing) => {
        for key in missing {
            println!("Missing required field: {}", key);
        }
    }
}

// Check if any of specific keys exist
if values.has_any(&[key("api_key"), key("token"), key("credentials")]) {
    println!("Has authentication");
}
```

### ParameterDiff Structure

```rust
/// Represents the difference between two ParameterValues
pub struct ParameterDiff {
    /// Values that were added (not in old, present in new)
    pub added: HashMap<Key, Value>,
    
    /// Values that were removed (in old, not in new)
    pub removed: HashMap<Key, Value>,
    
    /// Values that changed: (old_value, new_value)
    pub changed: HashMap<Key, (Value, Value)>,
}

impl ParameterDiff {
    /// Check if there are any changes
    pub fn is_empty(&self) -> bool;
    
    /// Get total number of changes
    pub fn total_changes(&self) -> usize;
    
    /// Apply this diff to ParameterValues
    pub fn apply(&self, values: &mut ParameterValues);
    
    /// Reverse this diff (for undo)
    pub fn reverse(&self) -> Self;
}
```

---

## Summary

| Component | Purpose |
|-----------|---------|
| `Command` trait | Encapsulates reversible operation |
| `SetValueCommand` | Single value change |
| `ResetCommand` | Reset to default |
| `MacroCommand` | Batch commands as transaction |
| `HistoryManager` | Undo/redo stack management |
| `Transaction` | Builder for atomic multi-step operations |
| `Snapshot` | Full state capture |
| `SnapshotCommand` | Full state undo/redo |
| `HistoryContext` | Integrated context with history |
| `ParameterValues` | Low-level value storage with snapshot/diff |
| `ParameterSnapshot` | Serializable snapshot of values |
| `ParameterDiff` | Change tracking between snapshots |
