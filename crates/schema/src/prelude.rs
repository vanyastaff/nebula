//! Common imports for schema-definition code.
//!
//! Bring this into scope with `use nebula_schema::prelude::*;` to get
//! everything needed to define and validate schemas without spelling out
//! each import individually.

pub use nebula_validator::Rule;

pub use crate::{
    BooleanField, CodeField, ComputedField, DynamicField, Expression, ExpressionContext,
    ExpressionMode, Field, FieldKey, FieldPath, FieldValue, FieldValues, FileField, InputHint,
    ListField, LoaderContext, LoaderRegistry, ModeField, NumberField, ObjectField, RequiredMode,
    ResolvedValues, Schema, SchemaBuilder, SecretField, SelectField, SelectOption, Severity,
    StringField, Transformer, ValidSchema, ValidValues, ValidationError, ValidationReport,
    VisibilityMode, field_key,
};
