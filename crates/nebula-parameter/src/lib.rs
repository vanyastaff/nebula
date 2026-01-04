#![allow(clippy::excessive_nesting)]

pub mod core;
pub mod error;
pub mod types;

// Re-export core functionality
pub use core::*;

// Re-export parameter types
pub use types::*;

// Re-export key types from nebula-core
pub use nebula_core::prelude::{KeyParseError, ParameterKey};

// Re-export conversion traits from nebula-value
pub use nebula_value::{JsonValueExt, ValueRefExt};

/// Prelude module for convenient imports
pub mod prelude {
    // Core traits
    pub use crate::core::{
        Describable, // base trait for parameter description
        Displayable, // display conditions
        Parameter,   // supertrait combining all
        Validatable, // validation
    };

    // Core types
    pub use crate::core::{
        ParameterCollection, // parameter registry
        ParameterContext,    // runtime context with reactive updates
        ParameterError,      // error type
        ParameterKind,       // parameter type enum
        ParameterMetadata,   // parameter metadata
        ParameterValues,     // value storage
    };

    // Display system
    pub use crate::core::{
        DisplayCondition, DisplayContext, DisplayRule, DisplayRuleSet, ParameterDisplay,
        ParameterDisplayError,
    };

    // Validation
    pub use crate::core::ParameterValidation;

    // State management
    pub use crate::core::{
        ParameterFlags,    // bitflags for parameter state
        ParameterSnapshot, // snapshot for saving/restoring
        ParameterState,    // runtime state of a parameter
    };

    // All parameter types
    pub use crate::types::*;

    // Re-exports from dependencies
    pub use nebula_core::prelude::{KeyParseError, ParameterKey};
    pub use nebula_value::ValueKind;
}
