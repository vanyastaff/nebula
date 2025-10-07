//! Built-in validators
//!
//! This module provides a comprehensive set of ready-to-use validators
//! for common validation scenarios.
//!
//! # Categories
//!
//! - **String**: Length, patterns, case, formats (email, URL, UUID)
//! - **Numeric**: Range, properties (even, odd, positive)
//! - **Collection**: Size, elements, structure
//! - **Logical**: Boolean, nullable
//! - **Bridge**: Legacy support for nebula-value::Value
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::validators::prelude::*;
//!
//! // String validation
//! let username = min_length(3).and(max_length(20)).and(alphanumeric());
//!
//! // Numeric validation
//! let age = in_range(18, 100);
//!
//! // Collection validation
//! let tags = min_size(1).and(max_size(10));
//!
//! // Composition
//! let email_validator = not_empty().and(email());
//! ```

pub mod collection;
pub mod logical;
pub mod numeric;
pub mod string;

// Re-export all validators
pub use collection::*;
pub use logical::*;
pub use numeric::*;
pub use string::*;


/// Prelude with all validators.
pub mod prelude {
    pub use super::collection::prelude::*;
    pub use super::logical::prelude::*;
    pub use super::numeric::prelude::*;
    pub use super::string::prelude::*;
}