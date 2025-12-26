---
name: rust-docs
description: Rust documentation generation and improvement. Use when documenting public APIs, creating examples, writing module-level docs, or improving existing documentation.
allowed-tools: Read, Write, Edit, Bash, Grep
version: 1.0.0
---

# Rust Documentation Standards

## Module-level Documentation

At the top of `lib.rs` or `mod.rs`:

```rust
//! # Module Name
//!
//! Brief one-line description of what this module does.
//!
//! ## Overview
//!
//! Longer explanation of the module's purpose, design decisions,
//! and how it fits into the larger system.
//!
//! ## Examples
//!
//! ```rust
//! use nebula_core::Module;
//!
//! let module = Module::new();
//! module.do_something()?;
//! ```
//!
//! ## Features
//!
//! - Feature 1: description
//! - Feature 2: description
```

## Function Documentation

```rust
/// Brief one-line description.
///
/// Longer explanation if needed. Explain what the function does,
/// not how it does it (that's what code is for).
///
/// # Arguments
///
/// * `param1` - Description of first parameter
/// * `param2` - Description of second parameter
///
/// # Returns
///
/// Description of the return value and its meaning.
///
/// # Errors
///
/// Describes when and why this function returns an error:
///
/// * [`Error::NotFound`] - When the item doesn't exist
/// * [`Error::InvalidInput`] - When input validation fails
///
/// # Panics
///
/// Describes conditions that cause a panic (if any).
///
/// # Examples
///
/// ```rust
/// use nebula_core::process;
///
/// let result = process(42, "test")?;
/// assert_eq!(result, expected);
/// ```
///
/// # Safety
///
/// (Only for unsafe functions) Explains invariants that must be upheld.
pub fn process(param1: i32, param2: &str) -> Result<Output, Error> {
    // ...
}
```

## Struct Documentation

```rust
/// A workflow execution context.
///
/// Contains all state needed to execute a workflow, including
/// configuration, credentials, and execution history.
///
/// # Examples
///
/// ```rust
/// use nebula_core::Context;
///
/// let ctx = Context::builder()
///     .workflow_id(id)
///     .timeout(Duration::from_secs(30))
///     .build()?;
/// ```
pub struct Context {
    /// Unique identifier for this workflow execution.
    pub id: ExecutionId,
    
    /// Maximum time allowed for execution.
    timeout: Duration,
    
    /// Credentials for external service access.
    credentials: Credentials,
}
```

## Enum Documentation

```rust
/// Possible states of a workflow execution.
///
/// Workflows transition through these states during their lifecycle.
/// See the [state machine diagram](crate::docs::state_machine) for details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    /// Workflow is queued but not yet started.
    Pending,
    
    /// Workflow is currently executing.
    ///
    /// Contains the timestamp when execution began.
    Running(Instant),
    
    /// Workflow completed successfully.
    Completed,
    
    /// Workflow failed with an error.
    ///
    /// Contains the error that caused the failure.
    Failed(ExecutionError),
}
```

## Trait Documentation

```rust
/// A storage backend for workflow state.
///
/// Implementations must be thread-safe and handle concurrent access.
/// All operations should be idempotent where possible.
///
/// # Implementing
///
/// ```rust
/// use nebula_core::Storage;
///
/// struct MyStorage { /* ... */ }
///
/// impl Storage for MyStorage {
///     async fn save(&self, id: &Id, data: &Data) -> Result<(), Error> {
///         // Your implementation
///     }
/// }
/// ```
pub trait Storage: Send + Sync {
    /// Saves data with the given identifier.
    ///
    /// Overwrites any existing data with the same ID.
    async fn save(&self, id: &Id, data: &Data) -> Result<(), Error>;
    
    /// Loads data by identifier.
    ///
    /// Returns `None` if no data exists for the given ID.
    async fn load(&self, id: &Id) -> Result<Option<Data>, Error>;
}
```

## Doc Test Patterns

### Basic Example
```rust
/// ```rust
/// let x = 5;
/// assert_eq!(x, 5);
/// ```
```

### Example with Error Handling
```rust
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let result = fallible_function()?;
/// assert!(result.is_valid());
/// # Ok(())
/// # }
/// ```
```

### Example Showing Error Case
```rust
/// ```rust,should_panic
/// let result = function_that_panics();
/// ```
```

### Example That Shouldn't Run
```rust
/// ```rust,no_run
/// // This connects to a real server
/// let client = Client::connect("production.example.com")?;
/// ```
```

### Example That Shouldn't Compile
```rust
/// ```rust,compile_fail
/// let x: i32 = "not a number"; // This won't compile
/// ```
```

## Link Patterns

```rust
/// Uses [`OtherType`] for processing.
/// See [`module::function`] for details.
/// Returns [`Result<T, Error>`](std::result::Result).
```

## Verification Commands

```bash
# Build documentation
cargo doc --no-deps --workspace

# Open in browser
cargo doc --no-deps --open

# Run doc tests
cargo test --doc --workspace

# Check for broken links
cargo doc --no-deps 2>&1 | grep -i "warning"

# Check documentation coverage (requires nightly)
RUSTDOCFLAGS="-Z unstable-options --show-coverage" cargo +nightly doc --no-deps
```

## Best Practices

1. **Write for the reader**: Assume they know Rust but not your code
2. **Examples are mandatory**: Every public item needs a working example
3. **Document errors**: List all error conditions for fallible functions
4. **Link generously**: Use `[`backticks`]` to link to related items
5. **Keep it current**: Update docs when code changes
6. **Use American English** for consistency
