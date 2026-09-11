//! Prelude module for convenient imports.
//!
//! Provides a single `use nebula_validator::prelude::*;` import that brings
//! in all commonly needed traits, types, validators, and combinators.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! // Extension method style - read left-to-right
//! "hello".validate_with(&min_length(3))?;
//! 42.validate_with(&min(10))?;
//!
//! // Direct method style - traditional
//! min_length(3).validate("hello")?;
//!
//! // Composition with combinators
//! let validator = min_length(3).and(max_length(20));
//! "hello".validate_with(&validator)?;
//! # Ok::<(), ValidationError>(())
//! ```

pub use crate::validators::*;
pub use crate::{
    combinators::{
        AllOf, And, AnyOf, CollectJsonFields, CollectionNested, Each, Field, JsonField, MultiField,
        NestedValidate, Not, OptionalNested, Or, SelfValidating, When, all_of, and, any_of,
        collect_json_fields, collection_nested, each, field, json_field, json_field_optional,
        named_field, nested_validator, not, optional_nested, or,
    },
    engine::{
        DeferredReason, EvaluationOutcome, ExecutionMode, validate_rules, validate_rules_with_ctx,
    },
    error::ValidatorError,
    foundation::{
        AnyValidator, AsValidatable, ErrorSeverity, FieldPath, FieldPathError, Validatable,
        Validate, ValidateExt, ValidationError, ValidationErrorKind, ValidationErrors,
        ValidationMode,
    },
    proof::Validated,
    rule::{
        DeferredRule, MAX_RULE_DEPTH, MAX_RULE_JSON_DEPTH, MAX_RULE_JSON_NODES, MAX_RULE_NODES,
        MAX_RULE_OPERANDS, MAX_RULE_TEXT_BYTES, Predicate, PredicateContext, Rule, RuleBudget,
        RuleBuildError, RuleChildren, RuleKind, RuleOperands, RulePattern, RuleRef, RuleView,
        ValueRule,
    },
};
