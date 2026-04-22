//! Common imports for schema-definition code.
//!
//! Bring this into scope with `use nebula_schema::prelude::*;` to get
//! everything needed to define and validate schemas without spelling out
//! each import individually.
//!
//! Covers:
//! - All 13 `Field` variants (`StringField`, `SecretField`, `NumberField`, `BooleanField`,
//!   `SelectField`, `ObjectField`, `ListField`, `ModeField`, `CodeField`, `FileField`,
//!   `ComputedField`, `DynamicField`, `NoticeField`) and their associated enums (`ComputedReturn`,
//!   `ModeVariant`, `NoticeSeverity`).
//! - The closure-style DSL trait (`FieldCollector`) so `.string()/.select()/…` are discoverable on
//!   `SchemaBuilder` without a separate import.
//! - The derive family: `HasSchema` / `HasSelectOptions` traits, the `EnumSelect` derive macro, and
//!   the `field_key!` macro. The `Schema` derive macro lives at `nebula_schema::Schema` — the same
//!   path as the `Schema` aggregate type (Rust allows a type and a derive macro to share a name);
//!   it isn't re-exported here because a prelude can't hold both meanings of the same identifier.
//! - `Rule` + `Predicate` for `visible_when` / `required_when` / `active_when`.

pub use nebula_schema_macros::EnumSelect;
pub use nebula_validator::{Predicate, Rule};

pub use crate::{
    BooleanField, CodeField, ComputedField, ComputedReturn, DynamicField, Expression,
    ExpressionContext, ExpressionMode, Field, FieldKey, FieldPath, FieldValue, FieldValues,
    FileField, HasSchema, HasSelectOptions, InputHint, KdfParams, ListField, LoaderContext,
    LoaderRegistry, ModeField, ModeVariant, NoticeField, NoticeSeverity, NumberField, ObjectField,
    RequiredMode, ResolvedValues, Schema, SchemaBuilder, SecretField, SecretValue, SecretWire,
    SelectField, SelectOption, Severity, StringField, Transformer, ValidSchema, ValidValues,
    ValidationError, ValidationReport, VisibilityMode, builder::FieldCollector, field_key,
};

#[cfg(test)]
mod coverage_smoke {
    //! Fails to compile if an item listed in the prelude doc comment stops
    //! being re-exported. Add any newly-documented item here.

    #[allow(unused_imports)]
    use super::*;

    #[allow(dead_code)]
    fn touch_all_reexports() {
        fn _j<T: HasSchema>(_: &T) {}
        fn _k<T: HasSelectOptions>(_: &T) {}
        fn _l<T: FieldCollector>(_: T) {}

        // Field variants.
        fn _f(_: &StringField, _: &SecretField, _: &NumberField, _: &BooleanField) {}
        fn _g(_: &SelectField, _: &ObjectField, _: &ListField, _: &ModeField) {}
        fn _h(_: &CodeField, _: &FileField, _: &ComputedField, _: &DynamicField) {}
        fn _i(_: &NoticeField) {}
        // Field-variant companions.
        let _: Option<NoticeSeverity> = None;
        let _: Option<ComputedReturn> = None;
        let _: Option<ModeVariant> = None;
        // Derive family (traits + `EnumSelect` macro is only touched at use sites).
        // Rule-building.
        let _: Option<Rule> = None;
        let _: Option<Predicate> = None;
        // Secret / KDF.
        let _: Option<KdfParams> = None;
        let _: Option<SecretValue> = None;
        {
            use crate::SecretString;
            let s = SecretValue::String(SecretString::new("prelude-coverage".to_owned()));
            let _w = SecretWire(&s);
        }
        // DSL trait.
    }
}
