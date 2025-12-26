---
name: rust-refactor
description: Rust code refactoring with API compatibility preservation. Use when improving code structure, eliminating duplication, optimizing, or restructuring modules.
allowed-tools: Read, Write, Edit, Bash, Grep, Glob
version: 1.0.0
---

# Rust Refactoring Patterns

## Safe Refactoring Process

1. **Verify baseline**: `cargo test --workspace` must pass
2. **Apply changes incrementally** - small steps
3. **After each step**: `cargo check && cargo test`
4. **Final verification**: `cargo clippy --workspace -- -D warnings`

## Common Refactorings

### Extract Function
```rust
// Before
fn process() {
    // ... 20 lines of validation ...
    // ... actual processing ...
}

// After
fn validate(input: &Input) -> Result<(), Error> {
    // ... validation logic ...
}

fn process() {
    validate(&input)?;
    // ... actual processing ...
}
```

### Builder Pattern
```rust
// Before
impl Config {
    pub fn new(host: String, port: u16, timeout: Duration, retries: u32) -> Self { ... }
}

// After
#[derive(Default)]
pub struct ConfigBuilder {
    host: Option<String>,
    port: Option<u16>,
    timeout: Option<Duration>,
    retries: Option<u32>,
}

impl ConfigBuilder {
    pub fn new() -> Self { Self::default() }
    
    pub fn host(mut self, h: impl Into<String>) -> Self {
        self.host = Some(h.into());
        self
    }
    
    pub fn port(mut self, p: u16) -> Self {
        self.port = Some(p);
        self
    }
    
    pub fn build(self) -> Result<Config, Error> {
        Ok(Config {
            host: self.host.ok_or(Error::MissingHost)?,
            port: self.port.unwrap_or(8080),
            timeout: self.timeout.unwrap_or(Duration::from_secs(30)),
            retries: self.retries.unwrap_or(3),
        })
    }
}
```

### Newtype Pattern
```rust
// Before - primitive obsession
fn process_workflow(id: Uuid, name: String) { ... }

// After - type safety
pub struct WorkflowId(Uuid);
pub struct WorkflowName(String);

impl WorkflowId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
    pub fn as_uuid(&self) -> &Uuid { &self.0 }
}

fn process_workflow(id: WorkflowId, name: WorkflowName) { ... }
```

### Error Consolidation
```rust
// Before - scattered errors
fn parse() -> Result<Data, ParseError> { ... }
fn validate() -> Result<(), ValidationError> { ... }
fn save() -> Result<(), IoError> { ... }

// After - unified error
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("parse failed: {0}")]
    Parse(#[from] ParseError),
    
    #[error("validation failed: {0}")]
    Validation(#[from] ValidationError),
    
    #[error("save failed: {0}")]
    Io(#[from] IoError),
}

fn process() -> Result<(), ProcessError> {
    let data = parse()?;
    validate(&data)?;
    save(&data)?;
    Ok(())
}
```

### Replace Conditional with Polymorphism
```rust
// Before
fn execute(action: &str, data: &Data) {
    match action {
        "create" => { /* 20 lines */ }
        "update" => { /* 20 lines */ }
        "delete" => { /* 20 lines */ }
        _ => panic!("unknown action"),
    }
}

// After
trait Action {
    fn execute(&self, data: &Data) -> Result<(), Error>;
}

struct CreateAction;
struct UpdateAction;
struct DeleteAction;

impl Action for CreateAction {
    fn execute(&self, data: &Data) -> Result<(), Error> { ... }
}
```

### Extract Trait
```rust
// Before - concrete dependency
fn process(storage: &PostgresStorage) { ... }

// After - abstraction
trait Storage {
    fn save(&self, data: &Data) -> Result<(), Error>;
    fn load(&self, id: &Id) -> Result<Data, Error>;
}

fn process(storage: &impl Storage) { ... }
```

## Rust-Specific Refactorings

### Prefer &str over String in Parameters
```rust
// Before
fn greet(name: String) { println!("Hello, {name}"); }

// After
fn greet(name: &str) { println!("Hello, {name}"); }
```

### Use impl Trait for Return Types
```rust
// Before
fn get_items() -> Vec<Item> { ... }

// After (when caller only iterates)
fn get_items() -> impl Iterator<Item = Item> { ... }
```

### Cow for Zero-Copy
```rust
use std::borrow::Cow;

fn process(input: Cow<'_, str>) -> Cow<'_, str> {
    if needs_modification(&input) {
        Cow::Owned(modify(&input))
    } else {
        input
    }
}
```

## Verification Commands

```bash
# Check compilation
cargo check --workspace

# Run all tests
cargo test --workspace

# Clippy checks
cargo clippy --workspace -- -D warnings

# Format check
cargo fmt --all -- --check

# Find unused dependencies
cargo machete

# Check for breaking changes (if publishing)
cargo semver-checks
```

## Nebula-specific Guidelines

- Maintain layer boundaries (core -> services -> api)
- Use `pub(crate)` by default, `pub` only for API
- Keep async boundaries clean (don't mix sync/async in one function)
- Preserve error context when converting errors
