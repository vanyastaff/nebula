//! Error constructor macros
//!
//! This module provides macros to eliminate duplication in error constructor methods.
//! Previously ~500 lines of repetitive code, now ~50 lines of declarative macros.
//!
//! NOTE: These macros are not yet used but demonstrate the refactoring approach.
//! They are kept as infrastructure for future refactoring of error.rs.

#![allow(unused_macros)]
//!
//! # Design Rationale
//!
//! Instead of writing 63 nearly identical wrapper functions:
//! ```rust,ignore
//! pub fn validation(message: impl Into<String>) -> Self {
//!     Self::new(ErrorKind::Client(ClientError::validation(message)))
//! }
//! pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
//!     Self::new(ErrorKind::Client(ClientError::not_found(resource_type, resource_id)))
//! }
//! // ... 61 more identical patterns
//! ```
//!
//! We use declarative macros to generate them automatically:
//! ```rust,ignore
//! client_error!(validation, message: String);
//! client_error!(not_found, resource_type: String, resource_id: String);
//! ```
//!
//! # Benefits
//!
//! 1. **DRY**: Single source of truth for constructor pattern
//! 2. **Maintainability**: Easy to add new error types
//! 3. **Consistency**: All constructors follow same pattern
//! 4. **Readability**: Declarative intent, less boilerplate

/// Macro to generate client error constructors
///
/// # Example
/// ```rust,ignore
/// client_error!(validation, message: String);
/// // Expands to:
/// pub fn validation(message: impl Into<String>) -> Self {
///     Self::new(ErrorKind::Client(ClientError::validation(message)))
/// }
/// ```
macro_rules! client_error {
    // Single argument
    ($name:ident, $arg:ident: $ty:ty) => {
        #[doc = concat!("Create a client error: ", stringify!($name))]
        pub fn $name($arg: impl Into<$ty>) -> Self {
            Self::new(crate::kinds::ErrorKind::Client(
                crate::kinds::ClientError::$name($arg.into())
            ))
        }
    };

    // Two arguments
    ($name:ident, $arg1:ident: $ty1:ty, $arg2:ident: $ty2:ty) => {
        #[doc = concat!("Create a client error: ", stringify!($name))]
        pub fn $name($arg1: impl Into<$ty1>, $arg2: impl Into<$ty2>) -> Self {
            Self::new(crate::kinds::ErrorKind::Client(
                crate::kinds::ClientError::$name($arg1.into(), $arg2.into())
            ))
        }
    };

    // Three arguments
    ($name:ident, $arg1:ident: $ty1:ty, $arg2:ident: $ty2:ty, $arg3:ident: $ty3:ty) => {
        #[doc = concat!("Create a client error: ", stringify!($name))]
        pub fn $name($arg1: impl Into<$ty1>, $arg2: impl Into<$ty2>, $arg3: impl Into<$ty3>) -> Self {
            Self::new(crate::kinds::ErrorKind::Client(
                crate::kinds::ClientError::$name($arg1.into(), $arg2.into(), $arg3.into())
            ))
        }
    };
}

/// Macro to generate server error constructors
macro_rules! server_error {
    ($name:ident, $arg:ident: $ty:ty) => {
        #[doc = concat!("Create a server error: ", stringify!($name))]
        pub fn $name($arg: impl Into<$ty>) -> Self {
            Self::new(crate::kinds::ErrorKind::Server(
                crate::kinds::ServerError::$name($arg.into())
            ))
        }
    };

    ($name:ident, $arg1:ident: $ty1:ty, $arg2:ident: $ty2:ty) => {
        #[doc = concat!("Create a server error: ", stringify!($name))]
        pub fn $name($arg1: impl Into<$ty1>, $arg2: impl Into<$ty2>) -> Self {
            Self::new(crate::kinds::ErrorKind::Server(
                crate::kinds::ServerError::$name($arg1.into(), $arg2.into())
            ))
        }
    };
}

/// Macro to generate system error constructors
macro_rules! system_error {
    ($name:ident, $arg:ident: $ty:ty) => {
        #[doc = concat!("Create a system error: ", stringify!($name))]
        pub fn $name($arg: impl Into<$ty>) -> Self {
            Self::new(crate::kinds::ErrorKind::System(
                crate::kinds::SystemError::$name($arg.into())
            ))
        }
    };

    ($name:ident, $arg1:ident: $ty1:ty, $arg2:ident: $ty2:ty) => {
        #[doc = concat!("Create a system error: ", stringify!($name))]
        pub fn $name($arg1: impl Into<$ty1>, $arg2: impl Into<$ty2>) -> Self {
            Self::new(crate::kinds::ErrorKind::System(
                crate::kinds::SystemError::$name($arg1.into(), $arg2.into())
            ))
        }
    };
}

/// Macro to generate memory error constructors
macro_rules! memory_error {
    ($name:ident, $arg:ident: $ty:ty) => {
        #[doc = concat!("Create a memory error: ", stringify!($name))]
        pub fn $name($arg: impl Into<$ty>) -> Self {
            Self::new(crate::kinds::ErrorKind::Memory(
                crate::kinds::MemoryError::$name($arg.into())
            ))
        }
    };

    ($name:ident, $arg1:ident: $ty1:ty, $arg2:ident: $ty2:ty) => {
        #[doc = concat!("Create a memory error: ", stringify!($name))]
        pub fn $name($arg1: impl Into<$ty1>, $arg2: impl Into<$ty2>) -> Self {
            Self::new(crate::kinds::ErrorKind::Memory(
                crate::kinds::MemoryError::$name($arg1.into(), $arg2.into())
            ))
        }
    };
}

/// Macro to generate resource error constructors
macro_rules! resource_error {
    ($name:ident, $arg:ident: $ty:ty) => {
        #[doc = concat!("Create a resource error: ", stringify!($name))]
        pub fn $name($arg: impl Into<$ty>) -> Self {
            Self::new(crate::kinds::ErrorKind::Resource(
                crate::kinds::ResourceError::$name($arg.into())
            ))
        }
    };

    ($name:ident, $arg1:ident: $ty1:ty, $arg2:ident: $ty2:ty) => {
        #[doc = concat!("Create a resource error: ", stringify!($name))]
        pub fn $name($arg1: impl Into<$ty1>, $arg2: impl Into<$ty2>) -> Self {
            Self::new(crate::kinds::ErrorKind::Resource(
                crate::kinds::ResourceError::$name($arg1.into(), $arg2.into())
            ))
        }
    };
}

// Export macros for use in error.rs
#[allow(unused_imports)]
pub(super) use client_error;
#[allow(unused_imports)]
pub(super) use memory_error;
#[allow(unused_imports)]
pub(super) use resource_error;
#[allow(unused_imports)]
pub(super) use server_error;
#[allow(unused_imports)]
pub(super) use system_error;
