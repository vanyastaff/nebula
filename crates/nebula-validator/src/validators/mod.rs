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
//! - **Bridge**: Legacy support for `nebula-value::Value`
//!
//! # Examples
//!
//! ```rust,ignore
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
pub mod network;
pub mod numeric;
pub mod string;
pub mod text;
pub mod value;

// Re-export all validators
#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use collection::*;
#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use logical::*;
#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use network::*;
#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use numeric::*;
#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use string::*;
#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use text::*;
#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use value::*;

/// Prelude with all validators.
pub mod prelude {
    #[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
    pub use super::collection::prelude::*;
    #[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
    pub use super::logical::prelude::*;
    #[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
    pub use super::network::*;
    #[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
    pub use super::numeric::prelude::*;
    #[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
    pub use super::string::prelude::*;
    #[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
    pub use super::text::*;
}
