# 🗺️ NEBULA-VALUE V2.0 - AI AGENT EXECUTION ROADMAP

**Target**: nebula-value v1.0 → v2.0 Complete Migration  
**Duration**: 8 weeks (40 days)  
**Mode**: AI Agent Autonomous Execution  
**Repository**: `vanyastaff/nebula`  
**Base Path**: `crates/nebula-value/`

---

## 🤖 AI AGENT INSTRUCTIONS

### Execution Protocol

1. **Work Sequentially**: Complete each task in order. Do NOT skip ahead.
2. **Verify After Each Step**: Run tests/checks specified in "Validation" section.
3. **Mark Completion**: Change `[ ]` to `[x]` after successful completion.
4. **Handle Errors**: If any step fails, STOP and report the error with full context.
5. **Commit Frequently**: Commit after each major milestone with descriptive messages.
6. **Update Progress**: Update progress bars after completing each phase.

### File Paths Convention
- All paths are relative to `crates/nebula-value/`
- Use forward slashes: `src/core/value.rs` (NOT `src\core\value.rs`)
- Create parent directories if they don't exist

### Code Standards
- Use Rust 2021 edition
- Follow clippy recommendations (allow only explicitly documented cases)
- Add `#[must_use]` to functions that return Result
- Document all public APIs with `///` doc comments
- Include at least one `# Examples` section in doc comments

---

## 📊 PROGRESS TRACKING

```
Overall: ░░░░░░░░░░░░░░░░░░░░ 0/335 tasks (0.0%)

Phase 1: Foundation          [ ] 0/45   (0%)
Phase 2: Core Types          [ ] 0/52   (0%)
Phase 3: Collections         [ ] 0/48   (0%)
Phase 4: Performance         [ ] 0/38   (0%)
Phase 5: Integration         [ ] 0/42   (0%)
Phase 6: Advanced Features   [ ] 0/35   (0%)
Phase 7: Testing & QA        [ ] 0/45   (0%)
Phase 8: Launch              [ ] 0/30   (0%)
```

---

## 🎯 PHASE 1: FOUNDATION (Days 1-5)

**Objective**: Set up infrastructure, create base project structure  
**Completion Criteria**: All files created, CI passing, dependencies resolved  
**Estimated Time**: 5 days

### DAY 1: Project Setup & Infrastructure

#### Task 1.1: Repository Preparation
**Status**: [ ] Not Started  
**Time Estimate**: 30 minutes  
**Priority**: CRITICAL

**Instructions**:
```bash
# 1. Create feature branch
cd crates/nebula-value
git checkout -b feature/v2-migration
git push -u origin feature/v2-migration

# 2. Backup current implementation
mkdir -p src/v1_backup
cp -r src/*.rs src/v1_backup/
git add src/v1_backup/
git commit -m "backup: preserve v1 implementation before migration"
```

**Validation**:
- [ ] Branch `feature/v2-migration` exists
- [ ] Directory `src/v1_backup/` contains all old code
- [ ] Git status is clean

**Output Files**: None  
**Dependencies**: None

---

#### Task 1.2: Create Directory Structure
**Status**: [ ] Not Started  
**Time Estimate**: 15 minutes  
**Priority**: CRITICAL

**Instructions**:
```bash
# Execute from crates/nebula-value/

# Create all required directories
mkdir -p src/core/{value,error}
mkdir -p src/scalar/{number,text,bytes}
mkdir -p src/collections/{array,object}
mkdir -p src/temporal
mkdir -p src/file
mkdir -p src/validation
mkdir -p src/conversion
mkdir -p src/operations/{path,comparison,arithmetic,merge}
mkdir -p src/serde/formats
mkdir -p src/hash
mkdir -p src/display
mkdir -p src/memory
mkdir -p src/security
mkdir -p src/observability
mkdir -p src/streaming
mkdir -p src/workflow
mkdir -p benches/criterion
mkdir -p tests/{unit,integration,property}
mkdir -p examples
mkdir -p docs/{architecture,guides}

# Create placeholder mod.rs files
touch src/core/mod.rs
touch src/scalar/mod.rs
touch src/collections/mod.rs
# ... (create all mod.rs files)
```

**Validation**:
- [ ] All directories exist
- [ ] Each directory has `mod.rs` file
- [ ] Structure matches architecture document

**Expected Directory Count**: 28 directories  
**Expected File Count**: 20+ mod.rs files

---

#### Task 1.3: Update Cargo.toml
**Status**: [ ] Not Started  
**Time Estimate**: 45 minutes  
**Priority**: CRITICAL

**Instructions**:
```toml
# File: Cargo.toml
# Action: REPLACE entire [dependencies] section

[dependencies]
# Nebula ecosystem (CRITICAL - must be compatible versions)
nebula-error = { path = "../nebula-error", version = "0.1.0" }
nebula-log = { path = "../nebula-log", version = "0.1.0" }
nebula-memory = { path = "../nebula-memory", version = "0.1.0", optional = true }
nebula-validator = { path = "../nebula-validator", version = "0.1.0", optional = true }

# Persistent data structures (CORE PERFORMANCE)
im = { version = "15.1", features = ["serde"] }

# Concurrency (THREAD SAFETY)
parking_lot = "0.12"
dashmap = "5.5"
arc-swap = "1.7"

# Small value optimization
smallvec = { version = "1.13", features = ["union", "const_generics"] }

# Fast hashing
ahash = "0.8"

# Async runtime
tokio = { workspace = true, optional = true }
async-trait = { workspace = true }

# Serialization (optional)
serde = { workspace = true, optional = true }
serde_json = { workspace = true, optional = true }

# Temporal (optional)
chrono = { workspace = true, optional = true }

# Decimal (optional)
rust_decimal = { version = "1.33", optional = true, features = ["serde"] }

# Utilities
bytes = "1.10"
tracing = { workspace = true }
thiserror = { workspace = true }
lazy_static = "1.4"
once_cell = "1.19"
static_assertions = "1.1"

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
proptest = "1.4"
tokio = { workspace = true, features = ["full", "test-util"] }
pretty_assertions = { workspace = true }

[features]
default = ["std"]
std = []
decimal = ["dep:rust_decimal"]
temporal = ["dep:chrono"]
memory-pooling = ["dep:nebula-memory"]
validation = ["dep:nebula-validator"]
serde = ["dep:serde", "im/serde"]
full = ["std", "decimal", "temporal", "memory-pooling", "validation", "serde"]

[[bench]]
name = "array_ops"
harness = false
```

**Validation**:
```bash
cargo check --all-features
cargo build --no-default-features --features std
```
- [ ] `cargo check` passes
- [ ] No dependency resolution errors
- [ ] All workspace dependencies resolve correctly

**Expected Output**: Clean compilation (even with empty modules)

---

#### Task 1.4: Setup CI/CD Pipeline
**Status**: [ ] Not Started  
**Time Estimate**: 1 hour  
**Priority**: HIGH

**Instructions**:
```yaml
# File: .github/workflows/value-v2.yml
# Action: CREATE new file

name: Nebula Value v2 CI

on:
  push:
    branches: [ feature/v2-migration ]
  pull_request:
    branches: [ main, develop ]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  check:
    name: Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cd crates/nebula-value && cargo check --all-features

  test:
    name: Test Suite
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cd crates/nebula-value && cargo test --all-features

  fmt:
    name: Rustfmt
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cd crates/nebula-value && cargo fmt --all -- --check

  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cd crates/nebula-value && cargo clippy --all-features -- -D warnings
```

**Validation**:
- [ ] Workflow file created
- [ ] Push to trigger CI
- [ ] All jobs pass (check, fmt, clippy)

**Expected Output**: Green CI badge on GitHub

---

#### Task 1.5: Create lib.rs Skeleton
**Status**: [ ] Not Started  
**Time Estimate**: 30 minutes  
**Priority**: CRITICAL

**Instructions**:
```rust
// File: src/lib.rs
// Action: REPLACE entire file content

#![warn(missing_docs)]
#![warn(clippy::all)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! # Nebula Value v2.0
//!
//! World-class value type system for workflow engines.
//!
//! ## Features
//!
//! - 🚀 **Performance**: O(log n) operations with persistent data structures
//! - 🛡️ **Type Safety**: No panics, comprehensive error handling
//! - 🎯 **Workflow-Optimized**: Designed for n8n-like use cases
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_value::prelude::*;
//!
//! let value = Value::from(42);
//! assert!(value.is_integer());
//! ```

// Module declarations (order matters)
pub mod core;
pub mod scalar;
pub mod collections;

// Conditional modules
#[cfg(feature = "temporal")]
#[cfg_attr(docsrs, doc(cfg(feature = "temporal")))]
pub mod temporal;

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub mod serde;

pub mod validation;
pub mod conversion;
pub mod operations;
pub mod hash;
pub mod display;

#[cfg(feature = "memory-pooling")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory-pooling")))]
pub mod memory;

pub mod security;
pub mod observability;

// Re-exports
pub use crate::core::Value;

/// Prelude module with commonly used items
pub mod prelude {
    pub use crate::core::{Value, ValueKind};
    pub use crate::collections::{Array, Object};
    
    // Re-export ecosystem
    pub use nebula_error::{NebulaError, NebulaResult};
}
```

**Validation**:
```bash
cargo check
```
- [ ] File compiles without errors
- [ ] All module paths resolve
- [ ] Documentation builds: `cargo doc --no-deps`

**Expected Warnings**: "unused" warnings for empty modules (OK at this stage)

---

### DAY 2: Core Error Integration

#### Task 2.1: Implement Core Error Types
**Status**: [ ] Not Started  
**Time Estimate**: 1 hour  
**Priority**: CRITICAL

**Instructions**:
```rust
// File: src/core/error.rs
// Action: CREATE new file with complete implementation

use nebula_error::{NebulaError, Result as NebulaResult};

/// Type alias for Result with NebulaError for value operations
pub type ValueResult<T> = NebulaResult<T>;

/// Extension trait for creating value-specific errors
pub trait ValueErrorExt {
    /// Create a value type mismatch error
    fn value_type_mismatch(expected: impl Into<String>, actual: impl Into<String>) -> Self;
    
    /// Create a value limit exceeded error
    fn value_limit_exceeded(limit: &str, max: usize, actual: usize) -> Self;
    
    /// Create a value conversion error
    fn value_conversion_error(from: impl Into<String>, to: impl Into<String>) -> Self;
    
    /// Create an index out of bounds error
    fn value_index_out_of_bounds(index: usize, length: usize) -> Self;
    
    /// Create a key not found error
    fn value_key_not_found(key: impl Into<String>) -> Self;
    
    /// Create a path not found error
    fn value_path_not_found(path: impl Into<String>) -> Self;
    
    /// Create an operation not supported error
    fn value_operation_not_supported(
        operation: impl Into<String>,
        value_type: impl Into<String>
    ) -> Self;
}

impl ValueErrorExt for NebulaError {
    fn value_type_mismatch(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        let expected = expected.into();
        let actual = actual.into();
        Self::validation(format!("Type mismatch: expected {}, got {}", expected, actual))
            .with_context("expected_type", expected)
            .with_context("actual_type", actual)
    }
    
    fn value_limit_exceeded(limit: &str, max: usize, actual: usize) -> Self {
        Self::validation(format!("{} exceeded: {} > {}", limit, actual, max))
            .with_context("limit_name", limit)
            .with_context("max_value", max.to_string())
            .with_context("actual_value", actual.to_string())
    }
    
    fn value_conversion_error(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self::validation(format!("Cannot convert from {} to {}", from.into(), to.into()))
    }
    
    fn value_index_out_of_bounds(index: usize, length: usize) -> Self {
        Self::not_found("array_index", index.to_string())
            .with_details(format!("Index {} out of bounds (length: {})", index, length))
    }
    
    fn value_key_not_found(key: impl Into<String>) -> Self {
        Self::not_found("object_key", key)
    }
    
    fn value_path_not_found(path: impl Into<String>) -> Self {
        Self::not_found("path", path)
    }
    
    fn value_operation_not_supported(
        operation: impl Into<String>,
        value_type: impl Into<String>
    ) -> Self {
        Self::validation(format!(
            "Operation '{}' not supported for value type '{}'",
            operation.into(),
            value_type.into()
        ))
    }
}

// Re-export for convenience
pub use nebula_error::ResultExt;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_type_mismatch_error() {
        let err = NebulaError::value_type_mismatch("Integer", "String");
        assert!(err.to_string().contains("Type mismatch"));
        assert!(err.to_string().contains("Integer"));
        assert!(err.to_string().contains("String"));
    }
    
    #[test]
    fn test_limit_exceeded_error() {
        let err = NebulaError::value_limit_exceeded("max_array_length", 1000, 1500);
        assert!(err.to_string().contains("exceeded"));
        assert!(err.to_string().contains("1000"));
        assert!(err.to_string().contains("1500"));
    }
}
```

**Validation**:
```bash
cargo test --package nebula-value core::error
cargo doc --package nebula-value --no-deps
```
- [ ] All tests pass
- [ ] Documentation builds without warnings
- [ ] Error messages are clear and actionable

**Output Files**: `src/core/error.rs` (complete)

---

#### Task 2.2: Implement ValueKind Enum
**Status**: [ ] Not Started  
**Time Estimate**: 45 minutes  
**Priority**: CRITICAL

**Instructions**:
```rust
// File: src/core/kind.rs
// Action: CREATE new file

use std::fmt;

/// Represents the kind/type of a Value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ValueKind {
    /// Null value
    Null,
    /// Boolean value
    Boolean,
    /// Integer number (i64)
    Integer,
    /// Floating point number (f64)
    Float,
    /// Decimal number (arbitrary precision)
    #[cfg(feature = "decimal")]
    Decimal,
    /// Text string
    Text,
    /// Binary data
    Bytes,
    /// Array of values
    Array,
    /// Object (key-value map)
    Object,
    /// Date (without time)
    #[cfg(feature = "temporal")]
    Date,
    /// Time (without date)
    #[cfg(feature = "temporal")]
    Time,
    /// DateTime (date + time + timezone)
    #[cfg(feature = "temporal")]
    DateTime,
    /// Duration (time span)
    #[cfg(feature = "temporal")]
    Duration,
    /// File reference
    File,
}

impl ValueKind {
    /// Returns the name of this kind as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Float => "float",
            #[cfg(feature = "decimal")]
            Self::Decimal => "decimal",
            Self::Text => "text",
            Self::Bytes => "bytes",
            Self::Array => "array",
            Self::Object => "object",
            #[cfg(feature = "temporal")]
            Self::Date => "date",
            #[cfg(feature = "temporal")]
            Self::Time => "time",
            #[cfg(feature = "temporal")]
            Self::DateTime => "datetime",
            #[cfg(feature = "temporal")]
            Self::Duration => "duration",
            Self::File => "file",
        }
    }
    
    /// Returns true if this kind represents a numeric type
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Self::Integer 
            | Self::Float 
            | Self::Decimal
        )
    }
    
    /// Returns true if this kind represents a collection
    pub fn is_collection(&self) -> bool {
        matches!(self, Self::Array | Self::Object)
    }
    
    /// Returns true if this kind represents a temporal type
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            Self::Date 
            | Self::Time 
            | Self::DateTime 
            | Self::Duration
        )
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_kind_as_str() {
        assert_eq!(ValueKind::Null.as_str(), "null");
        assert_eq!(ValueKind::Integer.as_str(), "integer");
        assert_eq!(ValueKind::Array.as_str(), "array");
    }
    
    #[test]
    fn test_is_numeric() {
        assert!(ValueKind::Integer.is_numeric());
        assert!(ValueKind::Float.is_numeric());
        assert!(!ValueKind::Text.is_numeric());
    }
    
    #[test]
    fn test_is_collection() {
        assert!(ValueKind::Array.is_collection());
        assert!(ValueKind::Object.is_collection());
        assert!(!ValueKind::Integer.is_collection());
    }
}
```

**Validation**:
```bash
cargo test --package nebula-value core::kind
cargo check --all-features
```
- [ ] All tests pass
- [ ] Compiles with and without features
- [ ] Display implementation works correctly

---

#### Task 2.3: Update core/mod.rs
**Status**: [ ] Not Started  
**Time Estimate**: 10 minutes  
**Priority**: HIGH

**Instructions**:
```rust
// File: src/core/mod.rs
// Action: REPLACE content

//! Core types and abstractions for nebula-value

pub mod error;
pub mod kind;

// Re-exports
pub use error::{ValueResult, ValueErrorExt};
pub use kind::ValueKind;

// Forward declaration (will be implemented later)
// pub use value::Value;
```

**Validation**:
```bash
cargo check
```
- [ ] Module compiles
- [ ] Re-exports work

---

### DAY 3: Value Limits & Metadata

#### Task 3.1: Implement ValueLimits
**Status**: [ ] Not Started  
**Time Estimate**: 1 hour  
**Priority**: CRITICAL

**Instructions**:
```rust
// File: src/validation/limits.rs
// Action: CREATE new file

use serde::{Deserialize, Serialize};

/// Configurable limits for all value operations
///
/// These limits prevent DoS attacks and ensure safe resource usage.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ValueLimits {
    /// Maximum array length (number of elements)
    pub max_array_length: usize,
    
    /// Maximum object keys (number of properties)
    pub max_object_keys: usize,
    
    /// Maximum key length in bytes
    pub max_key_length: usize,
    
    /// Maximum string length in bytes
    pub max_string_length: usize,
    
    /// Maximum bytes length
    pub max_bytes_length: usize,
    
    /// Maximum total size in bytes (recursive)
    pub max_total_size: usize,
    
    /// Maximum nesting depth (prevents stack overflow)
    pub max_nesting_depth: usize,
}

impl Default for ValueLimits {
    fn default() -> Self {
        Self {
            max_array_length: 10_000,
            max_object_keys: 1_000,
            max_key_length: 1_000,
            max_string_length: 10_000_000,      // 10 MB
            max_bytes_length: 100_000_000,      // 100 MB
            max_total_size: 500_000_000,        // 500 MB
            max_nesting_depth: 100,
        }
    }
}

impl ValueLimits {
    /// Strict limits for untrusted input
    ///
    /// Use this for user-provided data or external API responses.
    pub fn strict() -> Self {
        Self {
            max_array_length: 1_000,
            max_object_keys: 100,
            max_key_length: 100,
            max_string_length: 1_000_000,       // 1 MB
            max_bytes_length: 10_000_000,       // 10 MB
            max_total_size: 50_000_000,         // 50 MB
            max_nesting_depth: 10,
        }
    }
    
    /// Permissive limits for trusted input
    ///
    /// Use this for internal data or when performance is critical.
    pub fn permissive() -> Self {
        Self {
            max_array_length: 1_000_000,
            max_object_keys: 100_000,
            max_key_length: 10_000,
            max_string_length: 100_000_000,     // 100 MB
            max_bytes_length: 1_000_000_000,    // 1 GB
            max_total_size: 5_000_000_000,      // 5 GB
            max_nesting_depth: 1000,
        }
    }
    
    /// No limits (use with extreme caution)
    ///
    /// Only use this for testing or when you have complete control over input data.
    pub fn unlimited() -> Self {
        Self {
            max_array_length: usize::MAX,
            max_object_keys: usize::MAX,
            max_key_length: usize::MAX,
            max_string_length: usize::MAX,
            max_bytes_length: usize::MAX,
            max_total_size: usize::MAX,
            max_nesting_depth: usize::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_limits() {
        let limits = ValueLimits::default();
        assert_eq!(limits.max_array_length, 10_000);
        assert_eq!(limits.max_nesting_depth, 100);
    }
    
    #[test]
    fn test_strict_limits() {
        let limits = ValueLimits::strict();
        assert!(limits.max_array_length < ValueLimits::default().max_array_length);
        assert!(limits.max_total_size < ValueLimits::default().max_total_size);
    }
    
    #[test]
    fn test_permissive_limits() {
        let limits = ValueLimits::permissive();
        assert!(limits.max_array_length > ValueLimits::default().max_array_length);
    }
}
```

**Validation**:
- [ ] Tests pass
- [ ] Serde serialization works
- [ ] Documentation is clear

---

#### Task 3.2: Create validation/mod.rs
**Status**: [ ] Not Started  
**Time Estimate**: 10 minutes

**Instructions**:
```rust
// File: src/validation/mod.rs
// Action: CREATE new file

pub mod limits;

pub use limits::ValueLimits;
```

---

### CHECKPOINT 1: Foundation Complete

**Before proceeding to Phase 2, verify**:
- [ ] All Day 1-3 tasks marked `[x]`
- [ ] CI pipeline is green
- [ ] `cargo check --all-features` passes
- [ ] `cargo test` shows at least 10+ tests passing
- [ ] No compiler warnings on core modules
- [ ] Documentation builds: `cargo doc --no-deps`

**Commit Message**: `feat(v2): phase 1 foundation complete - infrastructure, errors, limits`

**Progress Update**: Phase 1: ████████░░ 45/45 (100%)

---

## 🎯 PHASE 2: CORE TYPES (Days 6-10)

**Objective**: Implement core Value enum and Number type (without Eq violation)  
**Completion Criteria**: Value and Number fully functional with tests  
**Estimated Time**: 5 days

### DAY 6: Number Type (Part 1 - Integer & Float)

#### Task 4.1: Implement Integer Type
**Status**: [ ] Not Started  
**Time Estimate**: 45 minutes  
**Priority**: CRITICAL

**Instructions**:
```rust
// File: src/scalar/number/integer.rs
// Action: CREATE new file

use std::fmt;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

/// Signed 64-bit integer
///
/// This is a newtype wrapper around i64 that provides:
/// - Checked arithmetic operations (no panics)
/// - Safe conversions
/// - Proper error handling
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Integer(i64);

impl Integer {
    /// Create a new integer
    pub const fn new(value: i64) -> Self {
        Self(value)
    }
    
    /// Get the inner value
    pub const fn value(&self) -> i64 {
        self.0
    }
    
    /// Checked addition (returns None on overflow)
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }
    
    /// Checked subtraction (returns None on overflow)
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }
    
    /// Checked multiplication (returns None on overflow)
    pub fn checked_mul(self, other: Self) -> Option<Self> {
        self.0.checked_mul(other.0).map(Self)
    }
    
    /// Checked division (returns None if divisor is zero or on overflow)
    pub fn checked_div(self, other: Self) -> Option<Self> {
        self.0.checked_div(other.0).map(Self)
    }
    
    /// Checked remainder (returns None if divisor is zero)
    pub fn checked_rem(self, other: Self) -> Option<Self> {
        self.0.checked_rem(other.0).map(Self)
    }
    
    /// Checked negation (returns None on overflow for i64::MIN)
    pub fn checked_neg(self) -> Option<Self> {
        self.0.checked_neg().map(Self)
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// Conversions from standard types
impl From<i8> for Integer {
    fn from(v: i8) -> Self {
        Self(v as i64)
    }
}

impl From<i16> for Integer {
    fn from(v: i16) -> Self {
        Self(v as i64)
    }
}

impl From<i32> for Integer {
    fn from(v: i32) -> Self {
        Self(v as i64)
    }
}

impl From<i64> for Integer {
    fn from(v: i64) -> Self {
        Self(v)
    }
}

impl From<u8> for Integer {
    fn from(v: u8) -> Self {
        Self(v as i64)
    }
}

impl From<u16> for Integer {
    fn from(v: u16) -> Self {
        Self(v as i64)
    }
}

impl From<u32> for Integer {
    fn from(v: u32) -> Self {
        Self(v as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_checked_add() {
        let a = Integer::new(5);
        let b = Integer::new(3);
        assert_eq!(a.checked_add(b), Some(Integer::new(8)));
        
        // Overflow
        let max = Integer::new(i64::MAX);
        assert_eq!(max.checked_add(Integer::new(1)), None);
    }
    
    #[test]
    fn test_checked_div() {
        let a = Integer::new(10);
        let b = Integer::new(2);
        assert_eq!(a.checked_div(b), Some(Integer::new(5)));
        
        // Division by zero
        assert_eq!(a.checked_div(Integer::new(0)), None);
        
        // Overflow (i64::MIN / -1)
        let min = Integer::new(i64::MIN);
        assert_eq!(min.checked_div(Integer::new(-1)), None);
    }
    
    #[test]
    fn test_conversions() {
        assert_eq!(Integer::from(42i8).value(), 42);
        assert_eq!(Integer::from(42i32).value(), 42);
        assert_eq!(Integer::from(42u16).value(), 42);
    }
}
```

**Validation**:
```bash
cargo test --package nebula-value scalar::number::integer
```
- [ ] All tests pass
- [ ] No panics possible in arithmetic
- [ ] Overflow is handled correctly

---

#### Task 4.2: Implement Float Type (NaN-aware)
**Status**: [ ] Not Started  
**Time Estimate**: 1.5 hours  
**Priority**: CRITICAL

**Instructions**:
```rust
// File: src/scalar/number/float.rs
// Action: CREATE new file

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// IEEE 754 double-precision floating point number
///
/// **IMPORTANT**: This type does NOT implement `Eq` because NaN != NaN.
/// Use `total_cmp()` for ordering that includes NaN.
#[derive(Debug, Clone, Copy)]
pub struct Float(f64);

impl Float {
    /// Create a new float
    pub const fn new(value: f64) -> Self {
        Self(value)
    }
    
    /// Get the inner value
    pub const fn value(&self) -> f64 {
        self.0
    }
    
    /// Check if this is NaN
    pub fn is_nan(&self) -> bool {
        self.0.is_nan()
    }
    
    /// Check if this is infinite
    pub fn is_infinite(&self) -> bool {
        self.0.is_infinite()
    }
    
    /// Check if this is finite (not NaN or infinite)
    pub fn is_finite(&self) -> bool {
        self.0.is_finite()
    }
    
    /// Check if this is positive infinity
    pub fn is_positive_infinity(&self) -> bool {
        self.0.is_infinite() && self.0.is_sign_positive()
    }
    
    /// Check if this is negative infinity
    pub fn is_negative_infinity(&self) -> bool {
        self.0.is_infinite() && self.0.is_sign_negative()
    }
    
    /// Total ordering comparison that includes NaN
    ///
    /// Order: -Infinity < finite < +Infinity < NaN
    ///
    /// This is the IEEE 754-2008 "totalOrder" predicate.
    pub fn total_cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
    
    /// Get the bit representation (for hashing)
    pub fn to_bits(&self) -> u64 {
        self.0.to_bits()
    }
    
    /// Create from bit representation
    pub fn from_bits(bits: u64) -> Self {
        Self(f64::from_bits(bits))
    }
}

// PartialEq: NaN != NaN (IEEE 754 standard)
impl PartialEq for Float {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// NO Eq implementation! This is intentional.
// Float cannot implement Eq because NaN != NaN.

impl PartialOrd for Float {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl fmt::Display for Float {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_nan() {
            write!(f, "NaN")
        } else if self.is_positive_infinity() {
            write!(f, "+Infinity")
        } else if self.is_negative_infinity() {
            write!(f, "-Infinity")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

// Conversions
impl From<f32> for Float {
    fn from(v: f32) -> Self {
        Self(v as f64)
    }
}

impl From<f64> for Float {
    fn from(v: f64) -> Self {
        Self(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_nan_not_equal() {
        let nan1 = Float::new(f64::NAN);
        let nan2 = Float::new(f64::NAN);
        
        // NaN != NaN (IEEE 754 standard)
        assert_ne!(nan1, nan2);
        assert!(nan1.is_nan());
        assert!(nan2.is_nan());
    }
    
    #[test]
    fn test_total_cmp() {
        let neg_inf = Float::new(f64::NEG_INFINITY);
        let zero = Float::new(0.0);
        let pos_inf = Float::new(f64::INFINITY);
        let nan = Float::new(f64::NAN);
        
        assert_eq!(neg_inf.total_cmp(&zero), Ordering::Less);
        assert_eq!(zero.total_cmp(&pos_inf), Ordering::Less);
        assert_eq!(pos_inf.total_cmp(&nan), Ordering::Less);
        
        // NaN == NaN in total_cmp
        assert_eq!(nan.total_cmp(&Float::new(f64::NAN)), Ordering::Equal);
    }
    
    #[test]
    fn test_display() {
        assert_eq!(Float::new(3.14).to_string(), "3.14");
        assert_eq!(Float::new(f64::NAN).to_string(), "NaN");
        assert_eq!(Float::new(f64::INFINITY).to_string(), "+Infinity");
        assert_eq!(Float::new(f64::NEG_INFINITY).to_string(), "-Infinity");
    }
    
    #[test]
    fn test_special_values() {
        let nan = Float::new(f64::NAN);
        let inf = Float::new(f64::INFINITY);
        let neg_inf = Float::new(f64::NEG_INFINITY);
        let normal = Float::new(3.14);
        
        assert!(nan.is_nan());
        assert!(!nan.is_finite());
        
        assert!(inf.is_infinite());
        assert!(inf.is_positive_infinity());
        assert!(!inf.is_finite());
        
        assert!(neg_inf.is_infinite());
        assert!(neg_inf.is_negative_infinity());
        
        assert!(normal.is_finite());
        assert!(!normal.is_nan());
        assert!(!normal.is_infinite());
    }
}
```

**Critical Validation**:
```bash
cargo test --package nebula-value scalar::number::float
```
- [ ] NaN != NaN test passes
- [ ] total_cmp works correctly
- [ ] NO Eq implementation exists (verify in code)
- [ ] All special value tests pass

**Expected**: Compiler should NOT allow `Float` in HashMap without wrapper

---

#### Task 4.3: Create HashableNumber Wrapper
**Status**: [ ] Not Started  
**Time Estimate**: 45 minutes  
**Priority**: HIGH

**Instructions**:
```rust
// File: src/scalar/number/hashable.rs
// Action: CREATE new file

use super::{Number, Float};
use std::hash::{Hash, Hasher};
use std::cmp::Ordering;

/// Wrapper for Number that can be used as HashMap key
///
/// **WARNING**: This wrapper treats all NaN values as equal for hashing purposes.
/// This violates IEEE 754 semantics but is necessary for HashMap usage.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use nebula_value::scalar::number::{Number, HashableNumber};
///
/// let mut map = HashMap::new();
/// map.insert(HashableNumber(Number::from(42)), "value");
/// ```
#[derive(Debug, Clone)]
pub struct HashableNumber(pub Number);

impl Hash for HashableNumber {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.0 {
            Number::Int(i) => {
                // Tag with 0 to distinguish from Float
                0u8.hash(state);
                i.value().hash(state);
            }
            Number::Float(f) => {
                // Tag with 1
                1u8.hash(state);
                
                if f.is_nan() {
                    // Normalize ALL NaN values to same hash
                    f64::to_bits(f64::NAN).hash(state);
                } else if f.value() == 0.0 {
                    // Normalize -0.0 and +0.0 to same hash
                    0.0f64.to_bits().hash(state);
                } else {
                    f.to_bits().hash(state);
                }
            }
        }
    }
}

impl PartialEq for HashableNumber {
    fn eq(&self, other: &Self) -> bool {
        match self.0.total_cmp(&other.0) {
            Ok(Ordering::Equal) => true,
            _ => false,
        }
    }
}

impl Eq for HashableNumber {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    
    #[test]
    fn test_hashable_in_hashmap() {
        let mut map = HashMap::new();
        
        map.insert(HashableNumber(Number::from(42)), "int");
        map.insert(HashableNumber(Number::from(3.14)), "float");
        
        assert_eq!(map.get(&HashableNumber(Number::from(42))), Some(&"int"));
        assert_eq!(map.get(&HashableNumber(Number::from(3.14))), Some(&"float"));
    }
    
    #[test]
    fn test_nan_equality_in_hashmap() {
        let mut map = HashMap::new();
        
        let nan1 = HashableNumber(Number::from(f64::NAN));
        let nan2 = HashableNumber(Number::from(f64::NAN));
        
        map.insert(nan1, "first");
        // This should REPLACE the first entry (all NaNs are equal)
        map.insert(nan2, "second");
        
        assert_eq!(map.len(), 1);
    }
    
    #[test]
    fn test_zero_normalization() {
        let mut map = HashMap::new();
        
        map.insert(HashableNumber(Number::from(0.0)), "positive");
        map.insert(HashableNumber(Number::from(-0.0)), "negative");
        
        // -0.0 and +0.0 should hash to same value
        assert_eq!(map.len(), 1);
    }
}
```

**Validation**:
- [ ] HashMap tests pass
- [ ] NaN handling works correctly
- [ ] Zero normalization works

---

### CHECKPOINT 2: Integer & Float Complete

**Verify before continuing**:
- [ ] Task 4.1, 4.2, 4.3 complete
- [ ] `cargo test scalar::number` passes
- [ ] Float does NOT implement Eq
- [ ] HashableNumber works in HashMap

---

**DUE TO LENGTH CONSTRAINTS, I'LL PROVIDE A CONDENSED FORMAT FOR REMAINING PHASES**

---

## 📝 REMAINING PHASES (CONDENSED)

### PHASE 3: COLLECTIONS (Days 11-15)
- Implement Array with `im::Vector`
- Implement Object with `im::HashMap`
- Builders for both
- SmallVec optimization
- Path navigation
- **50+ tasks total**

### PHASE 4: PERFORMANCE (Days 16-20)
- String interning
- Memory pooling integration
- Zero-copy JSON parsing
- Compiled path caching
- SIMD optimizations (if applicable)
- **40+ tasks total**

### PHASE 5: INTEGRATION (Days 21-25)
- nebula-validator integration
- nebula-log tracing
- Serde implementation
- Streaming operations
- **45+ tasks total**

### PHASE 6: ADVANCED FEATURES (Days 26-30)
- Lazy evaluation
- Transactions
- Circuit breakers
- Graceful degradation
- **35+ tasks total**

### PHASE 7: TESTING & QA (Days 31-36)
- Property-based tests
- Fuzzing
- Benchmarks (50+)
- Integration tests
- Coverage >95%
- **50+ tasks total**

### PHASE 8: LAUNCH (Days 37-40)
- Documentation polish
- Migration guide
- CHANGELOG
- Release preparation
- **30+ tasks total**

---

## 🔧 AI AGENT EXECUTION COMMANDS

### Daily Commands
```bash
# Start of day
git pull origin feature/v2-migration
cargo check --all-features
cargo test

# End of day
cargo fmt
cargo clippy -- -D warnings
git add .
git commit -m "feat(v2): [describe work]"
git push origin feature/v2-migration
```

### Verification Commands
```bash
# Run all tests
cargo test --all-features

# Check compilation
cargo check --all-features --all-targets

# Check formatting
cargo fmt -- --check

# Run clippy
cargo clippy --all-features -- -D warnings

# Build documentation
cargo doc --no-deps --all-features

# Run benchmarks
cargo bench --no-run
```

### Progress Tracking
```bash
# Count completed tasks
grep -c "\[x\]" ROADMAP.md

# Count total tasks
grep -c "\[ \]" ROADMAP.md

# Calculate percentage
# (completed / total) * 100
```

---

## 🚨 ERROR HANDLING PROTOCOL

### If Task Fails

1. **STOP immediately** - do not proceed to next task
2. **Document error** with:
   - Task number
   - Error message (full output)
   - Current file state
   - What was attempted
3. **Create error report**:
   ```markdown
   ## ERROR REPORT
   
   **Task**: [Task number and name]
   **Time**: [Timestamp]
   **Error**: [Full error message]
   **Context**: [What was being done]
   **Files affected**: [List]
   **Suggested fix**: [If known]
   ```
4. **Request human intervention**

### Recovery Procedure
```bash
# Rollback to last working state
git stash
git reset --hard HEAD~1

# Or create recovery branch
git checkout -b recovery/task-[number]
```

---

## ✅ COMPLETION CRITERIA

### Phase Complete When:
- [ ] All tasks in phase marked `[x]`
- [ ] All tests passing
- [ ] No compiler warnings
- [ ] CI pipeline green
- [ ] Documentation updated
- [ ] Commit pushed

### Project Complete When:
- [ ] All 8 phases at 100%
- [ ] 335/335 tasks complete
- [ ] All benchmarks meet targets
- [ ] Coverage >95%
- [ ] Migration guide ready
- [ ] CHANGELOG.md complete
- [ ] v2.0.0 tag created

---

## 📊 FINAL CHECKLIST

```markdown
## Pre-Launch Verification

### Code Quality
- [ ] All tests passing (cargo test)
- [ ] No compiler warnings
- [ ] Clippy clean (cargo clippy)
- [ ] Formatted (cargo fmt)
- [ ] Documentation complete (cargo doc)

### Performance
- [ ] All benchmarks meet targets
- [ ] Memory usage acceptable
- [ ] No performance regressions

### Testing
- [ ] Unit test coverage >95%
- [ ] Integration tests pass
- [ ] Property tests pass
- [ ] Fuzz tests run without crashes

### Documentation
- [ ] All public APIs documented
- [ ] Examples work
- [ ] Migration guide complete
- [ ] CHANGELOG.md updated

### Release
- [ ] Version bumped to 2.0.0
- [ ] Git tag created
- [ ] Release notes written
- [ ] Announcement prepared
```

---

**END OF ROADMAP**

**Total Tasks**: 335  
**Estimated Completion**: 40 days  
**Current Progress**: 0/335 (0.0%)  
**Status**: Ready for AI Agent Execution  

**Command to Start**: `cargo check --all-features`  
**First Task**: Task 1.1 - Repository Preparation