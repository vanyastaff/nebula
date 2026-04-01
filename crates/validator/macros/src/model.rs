//! Intermediate representation types for the Validator derive macro.
//!
//! These types represent parsed validation rules as data before codegen,
//! forming the bridge between the parse phase (attribute extraction) and
//! the emit phase (token stream generation).

#![allow(dead_code)] // Types unused until Task 4 wires the 3-phase pipeline.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream as TokenStream2;
use syn::{Generics, Ident, Type};

/// Top-level input for the Validator derive macro.
///
/// Contains all parsed information needed to generate the `Validate`,
/// `SelfValidating`, and `validate_fields()` implementations.
#[derive(Debug)]
pub struct ValidatorInput {
    /// The struct identifier.
    pub ident: Ident,
    /// Generic parameters from the struct definition.
    pub generics: Generics,
    /// Container-level attributes (`#[validator(...)]`).
    pub container: ContainerAttrs,
    /// Per-field definitions with their validation rules.
    pub fields: Vec<FieldDef>,
}

/// Container-level attributes parsed from `#[validator(...)]`.
#[derive(Debug)]
pub struct ContainerAttrs {
    /// Root error message used when converting `ValidationErrors` to a
    /// single `ValidationError`. Defaults to `"validation failed"`.
    pub message: String,
}

impl Default for ContainerAttrs {
    fn default() -> Self {
        Self {
            message: "validation failed".to_string(),
        }
    }
}

/// A single field definition with its validation rules.
#[derive(Debug)]
pub struct FieldDef {
    /// The field identifier.
    pub ident: Ident,
    /// The original type as written in the struct (may be `Option<T>`).
    pub ty: Type,
    /// Whether the field type is `Option<T>`.
    pub is_option: bool,
    /// The inner type unwrapped from `Option`, or the original type if not optional.
    pub inner_ty: Type,
    /// Per-field message override from `#[validate(message = "...")]`.
    pub message: Option<String>,
    /// Validation rules applied directly to the field value.
    pub rules: Vec<Rule>,
    /// Element-level rules for `Vec<T>` fields via `each(...)`.
    pub each_rules: Option<EachRules>,
    /// The span of the field for error reporting.
    pub span: proc_macro2::Span,
}

/// A single validation rule extracted from `#[validate(...)]` attributes.
#[derive(Debug)]
pub enum Rule {
    /// Field is required (must be `Some` for `Option` fields).
    Required,

    /// Minimum string/collection length: `min_length = N`.
    MinLength(usize),
    /// Maximum string/collection length: `max_length = N`.
    MaxLength(usize),
    /// Exact string/collection length: `exact_length = N`.
    ExactLength(usize),
    /// String length range: `length_range(min = N, max = M)`.
    LengthRange {
        /// Minimum length (inclusive).
        min: usize,
        /// Maximum length (inclusive).
        max: usize,
    },

    /// Minimum numeric value: `min = N`.
    Min(TokenStream2),
    /// Maximum numeric value: `max = N`.
    Max(TokenStream2),

    /// Minimum collection size: `min_size = N`.
    MinSize(usize),
    /// Maximum collection size: `max_size = N`.
    MaxSize(usize),
    /// Exact collection size: `exact_size = N`.
    ExactSize(usize),
    /// Collection size range: `size_range(min = N, max = M)`.
    SizeRange {
        /// Minimum size (inclusive).
        min: usize,
        /// Maximum size (inclusive).
        max: usize,
    },

    /// Collection must not be empty: `not_empty_collection`.
    NotEmptyCollection,

    /// Built-in string format validator (zero-arg factory).
    StringFormat(StringFormat),
    /// Built-in string factory validator (one-arg factory).
    StringFactory {
        /// The factory kind (contains, starts_with, ends_with).
        kind: StringFactoryKind,
        /// The string argument passed to the factory.
        arg: String,
    },

    /// Boolean must be true: `is_true`.
    IsTrue,
    /// Boolean must be false: `is_false`.
    IsFalse,

    /// Regex pattern match: `regex = "pattern"`.
    Regex(String),

    /// Nested validation via `SelfValidating::check`: `nested`.
    Nested,

    /// Custom validator expression: `custom = path_or_closure`.
    Custom(TokenStream2),
}

/// Element-level validation rules for `Vec<T>` fields via `each(...)`.
#[derive(Debug)]
pub struct EachRules {
    /// The element type extracted from `Vec<T>`.
    pub element_ty: Type,
    /// Validation rules applied to each element.
    pub rules: Vec<Rule>,
}

/// Built-in string format validators (zero-argument factories).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringFormat {
    /// String must not be empty.
    NotEmpty,
    /// String must be alphanumeric.
    Alphanumeric,
    /// String must be alphabetic.
    Alphabetic,
    /// String must be numeric.
    Numeric,
    /// String must be lowercase.
    Lowercase,
    /// String must be uppercase.
    Uppercase,
    /// String must be a valid email address.
    Email,
    /// String must be a valid URL.
    Url,
    /// String must be a valid IPv4 address.
    Ipv4,
    /// String must be a valid IPv6 address.
    Ipv6,
    /// String must be a valid IP address (v4 or v6).
    IpAddr,
    /// String must be a valid hostname.
    Hostname,
    /// String must be a valid UUID.
    Uuid,
    /// String must be a valid date.
    Date,
    /// String must be a valid date-time.
    DateTime,
    /// String must be a valid time.
    Time,
}

/// Built-in string factory validators (one-argument factories).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringFactoryKind {
    /// String must contain the given substring.
    Contains,
    /// String must start with the given prefix.
    StartsWith,
    /// String must end with the given suffix.
    EndsWith,
}
