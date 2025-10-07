//! Procedural macros for the Nebula workflow engine
//!
//! This crate provides derive macros for various Nebula components:
//!
//! - **`#[derive(Validator)]`** - Automatic validator implementation
//! - **`#[derive(Parameter)]`** - Parameter builder generation (future)
//! - **`#[derive(Action)]`** - Action trait implementation (future)
//! - **`#[derive(Resource)]`** - Resource management (future)
//!
//! # Examples
//!
//! ## Validator Derive
//!
//! ```rust,ignore
//! use nebula_derive::Validator;
//! use nebula_validator::prelude::*;
//!
//! #[derive(Validator)]
//! struct UserInput {
//!     #[validate(min_length = 3, max_length = 20, alphanumeric)]
//!     username: String,
//!
//!     #[validate(email)]
//!     email: String,
//!
//!     #[validate(range(min = 18, max = 100))]
//!     age: u8,
//! }
//! ```
//!
//! # Architecture
//!
//! Each derive macro is implemented in its own module:
//!
//! - `validator/` - Validator derive implementation
//! - `parameter/` - Parameter derive implementation (future)
//! - `action/` - Action derive implementation (future)
//! - `resource/` - Resource derive implementation (future)
//!
//! Shared utilities are in the `utils/` module.

use proc_macro::TokenStream;

// Module declarations
mod validator;
mod utils;

// Future modules (placeholder)
// mod parameter;
// mod action;
// mod resource;

// ============================================================================
// VALIDATOR DERIVE
// ============================================================================

/// Derives a validator implementation for a struct.
///
/// This macro generates validation logic for struct fields based on
/// `#[validate(...)]` attributes.
///
/// # Attributes
///
/// ## String Validators
///
/// - `#[validate(min_length = N)]` - Minimum length
/// - `#[validate(max_length = N)]` - Maximum length
/// - `#[validate(exact_length = N)]` - Exact length
/// - `#[validate(email)]` - Email format
/// - `#[validate(url)]` - URL format
/// - `#[validate(regex = "pattern")]` - Regex pattern
/// - `#[validate(alphanumeric)]` - Alphanumeric only
/// - `#[validate(contains = "substring")]` - Must contain substring
///
/// ## Numeric Validators
///
/// - `#[validate(range(min = N, max = M))]` - Range validation
/// - `#[validate(min = N)]` - Minimum value
/// - `#[validate(max = N)]` - Maximum value
/// - `#[validate(positive)]` - Must be positive
/// - `#[validate(negative)]` - Must be negative
/// - `#[validate(even)]` - Must be even
/// - `#[validate(odd)]` - Must be odd
///
/// ## Collection Validators
///
/// - `#[validate(min_size = N)]` - Minimum collection size
/// - `#[validate(max_size = N)]` - Maximum collection size
/// - `#[validate(unique)]` - All elements must be unique
/// - `#[validate(non_empty)]` - Must not be empty
///
/// ## Logical Validators
///
/// - `#[validate(required)]` - Field is required (not None)
/// - `#[validate(custom = "function_name")]` - Custom validation function
///
/// ## Composition
///
/// Multiple validators can be combined:
///
/// ```rust,ignore
/// #[validate(min_length = 5, max_length = 20, alphanumeric)]
/// username: String,
/// ```
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust,ignore
/// use nebula_derive::Validator;
/// use nebula_validator::prelude::*;
///
/// #[derive(Validator)]
/// struct LoginForm {
///     #[validate(email)]
///     email: String,
///
///     #[validate(min_length = 8)]
///     password: String,
/// }
///
/// let form = LoginForm {
///     email: "user@example.com".to_string(),
///     password: "secret123".to_string(),
/// };
///
/// // Validate the struct
/// form.validate()?;
/// ```
///
/// ## Custom Error Messages
///
/// ```rust,ignore
/// #[derive(Validator)]
/// struct UserRegistration {
///     #[validate(
///         min_length = 3,
///         message = "Username must be at least 3 characters"
///     )]
///     username: String,
/// }
/// ```
///
/// ## Nested Validation
///
/// ```rust,ignore
/// #[derive(Validator)]
/// struct Address {
///     #[validate(min_length = 1)]
///     street: String,
///
///     #[validate(min_length = 1)]
///     city: String,
/// }
///
/// #[derive(Validator)]
/// struct User {
///     #[validate(email)]
///     email: String,
///
///     #[validate(nested)]
///     address: Address,
/// }
/// ```
///
/// ## Custom Validators
///
/// ```rust,ignore
/// fn validate_username(username: &str) -> Result<(), ValidationError> {
///     if username.starts_with("admin") {
///         Err(ValidationError::new("invalid_username", "Cannot start with 'admin'"))
///     } else {
///         Ok(())
///     }
/// }
///
/// #[derive(Validator)]
/// struct UserForm {
///     #[validate(custom = "validate_username")]
///     username: String,
/// }
/// ```
#[proc_macro_derive(Validator, attributes(validate))]
pub fn derive_validator(input: TokenStream) -> TokenStream {
    validator::derive_validator_impl(input)
}

// ============================================================================
// FUTURE DERIVES (Placeholders)
// ============================================================================

// /// Derives a parameter builder for a struct.
// ///
// /// This macro generates a fluent builder API for creating validated parameters.
// #[proc_macro_derive(Parameter, attributes(parameter))]
// pub fn derive_parameter(input: TokenStream) -> TokenStream {
//     parameter::derive_parameter_impl(input)
// }

// /// Derives an action implementation for a struct.
// ///
// /// This macro implements the Action trait with automatic input/output handling.
// #[proc_macro_derive(Action, attributes(action))]
// pub fn derive_action(input: TokenStream) -> TokenStream {
//     action::derive_action_impl(input)
// }

// /// Derives resource management for a struct.
// ///
// /// This macro implements resource lifecycle management.
// #[proc_macro_derive(Resource, attributes(resource))]
// pub fn derive_resource(input: TokenStream) -> TokenStream {
//     resource::derive_resource_impl(input)
// }
