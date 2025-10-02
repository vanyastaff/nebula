#![allow(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::all)]
extern crate alloc;
pub mod collections;
pub mod core;
pub mod scalar;
#[cfg(feature = "temporal")]
pub mod temporal;
pub mod validation;
// Re-export core types
pub use core::{
    NebulaError, NebulaResult, ResultExt,
    error::{ValueErrorExt, ValueResult, ValueResultExt},
    limits::ValueLimits,
    value::Value,
};
// Re-export scalar and collection types
pub use collections::{Array, Object};
pub use scalar::{Bytes, Float, Integer, Text};

// Re-export temporal types
#[cfg(feature = "temporal")]
pub use temporal::{Date, DateTime, Duration, Time};

// Re-export serde_json::json! macro for convenience
#[cfg(feature = "serde")]
pub use serde_json::json;

// Re-export conversion extension traits for ergonomic usage
#[cfg(feature = "serde")]
pub use core::convert::{JsonValueExt, ValueRefExt};

/// Prelude for common imports
pub mod prelude {
    pub use crate::{Array, Bytes, Float, Integer, Object, Text};
    pub use crate::{NebulaError, Value, ValueErrorExt, ValueResult, ValueResultExt};

    #[cfg(feature = "temporal")]
    pub use crate::{Date, DateTime, Duration, Time};

    #[cfg(feature = "serde")]
    pub use serde_json::json;
}
