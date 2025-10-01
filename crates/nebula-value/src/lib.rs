#![allow(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::all)]
extern crate alloc;
pub mod core;
pub mod scalar;
pub mod collections;
#[cfg(feature = "temporal")]
pub mod temporal;
pub mod validation;
// Re-export core types
pub use core::{
    error::{ValueResult, ValueErrorExt, ValueResultExt},
    limits::ValueLimits,
    value::Value,
    NebulaError, NebulaResult, ResultExt,
};
// Re-export scalar and collection types
pub use scalar::{Integer, Float, Text, Bytes};
pub use collections::{Array, Object};

// Re-export temporal types
#[cfg(feature = "temporal")]
pub use temporal::{Date, Time, DateTime, Duration};

// Re-export serde_json::json! macro for convenience
#[cfg(feature = "serde")]
pub use serde_json::json;

// Re-export conversion extension traits for ergonomic usage
#[cfg(feature = "serde")]
pub use core::convert::{ValueRefExt, JsonValueExt};

/// Prelude for common imports
pub mod prelude {
    pub use crate::{Value, ValueResult, ValueErrorExt, ValueResultExt, NebulaError};
    pub use crate::{Integer, Float, Text, Bytes, Array, Object};

    #[cfg(feature = "temporal")]
    pub use crate::{Date, Time, DateTime, Duration};

    #[cfg(feature = "serde")]
    pub use serde_json::json;
}
