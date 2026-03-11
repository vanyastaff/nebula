//! Declarative validation rules.
//!
//! Re-exports the unified [`Rule`] type from the validator crate.
//! All rule variants — value validation, context predicates, and logical
//! combinators — are defined in one place.

pub use nebula_validator::rule::Rule;
