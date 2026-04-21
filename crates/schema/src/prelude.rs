//! Common imports for schema-definition code.
//!
//! Bring this into scope with `use nebula_schema::prelude::*;` to get
//! everything needed to define and validate schemas without spelling out
//! each import individually.
//!
//! Covers:
//! - All `Field` variants and their builders.
//! - The closure-style DSL trait (`FieldCollector`) so `.string()/.select()/…` are discoverable on
//!   `SchemaBuilder` without a separate import.
//! - The `HasSchema` trait for types produced by `#[derive(Schema)]`.
//! - `Rule` + `Predicate` for `visible_when` / `required_when` / `active_when`.

pub use nebula_validator::{Predicate, Rule};

pub use crate::{
    BooleanField, CodeField, ComputedField, DynamicField, Expression, ExpressionContext,
    ExpressionMode, Field, FieldKey, FieldPath, FieldValue, FieldValues, FileField, HasSchema,
    InputHint, ListField, LoaderContext, LoaderRegistry, ModeField, NumberField, ObjectField,
    RequiredMode, ResolvedValues, Schema, SchemaBuilder, SecretField, SelectField, SelectOption,
    Severity, StringField, Transformer, ValidSchema, ValidValues, ValidationError,
    ValidationReport, VisibilityMode, builder::FieldCollector, field_key,
};
