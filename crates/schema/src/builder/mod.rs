//! Typed-closure builder DSL on top of the per-type field structs.
//!
//! Leaf field builders are re-exports of the per-type structs — they already
//! enforce compile-time type safety via distinct method surfaces (e.g.
//! `StringField::min_length` exists but `BooleanField::min_length` does not).
//! The `Schema::builder().string(field_key!("k"), |s| ...)` closure form pins the argument
//! type, so a `.min_length` call on a `BooleanBuilder` is a compile error.
//!
//! Composite builders ([`ObjectBuilder`], [`ListBuilder`], [`GroupBuilder`])
//! are real types that expose nested closure-based child methods that the
//! leaf fields don't support.

mod group;
mod list;
mod object;

pub use group::GroupBuilder;
pub use list::ListBuilder;
pub use object::ObjectBuilder;

// Leaf builders — type aliases onto the existing per-type structs.
pub use crate::field::{
    BooleanField as BooleanBuilder, CodeField as CodeBuilder, NumberField as NumberBuilder,
    SecretField as SecretBuilder, SelectField as SelectBuilder, StringField as StringBuilder,
};
use crate::{
    field::{BooleanField, CodeField, Field, NumberField, SecretField, SelectField, StringField},
    key::FieldKey,
};

/// Collection sink for typed-closure child field methods.
///
/// Implemented by any builder that appends a single [`Field`] value per child
/// method call. Default method implementations provide the closure-based
/// `.string / .number / .boolean / .select / .code / .secret / .object / .list`
/// API that `SchemaBuilder`, `ObjectBuilder`, and `GroupBuilder` share.
///
/// All child methods take a pre-validated [`FieldKey`]. Use the
/// [`field_key!`](crate::field_key) macro for compile-time keys, or
/// [`FieldKey::new`](crate::FieldKey::new) for runtime-validated strings.
pub trait FieldCollector: Sized {
    /// Append a single built field to the underlying collection.
    #[doc(hidden)]
    #[must_use = "builder methods must be chained"]
    fn push_field(self, field: Field) -> Self;

    /// Append many built fields at once — works on any collector
    /// (`SchemaBuilder` / `ObjectBuilder` / `GroupBuilder`).
    #[must_use]
    fn extend<I, F>(self, fields: I) -> Self
    where
        I: IntoIterator<Item = F>,
        F: Into<Field>,
    {
        fields
            .into_iter()
            .fold(self, |acc, f| acc.push_field(f.into()))
    }

    /// Append a string child built via a typed closure.
    #[must_use]
    fn string(self, key: FieldKey, f: impl FnOnce(StringBuilder) -> StringBuilder) -> Self {
        self.push_field(f(StringField::new(key)).into())
    }

    /// Append a secret child built via a typed closure.
    #[must_use]
    fn secret(self, key: FieldKey, f: impl FnOnce(SecretBuilder) -> SecretBuilder) -> Self {
        self.push_field(f(SecretField::new(key)).into())
    }

    /// Append a number child built via a typed closure.
    #[must_use]
    fn number(self, key: FieldKey, f: impl FnOnce(NumberBuilder) -> NumberBuilder) -> Self {
        self.push_field(f(NumberField::new(key)).into())
    }

    /// Append an integer child (`NumberField` with `integer=true`) via a typed closure.
    #[must_use]
    fn integer(self, key: FieldKey, f: impl FnOnce(NumberBuilder) -> NumberBuilder) -> Self {
        self.push_field(f(NumberField::new(key).integer()).into())
    }

    /// Append a boolean child built via a typed closure.
    #[must_use]
    fn boolean(self, key: FieldKey, f: impl FnOnce(BooleanBuilder) -> BooleanBuilder) -> Self {
        self.push_field(f(BooleanField::new(key)).into())
    }

    /// Append a select child built via a typed closure.
    #[must_use]
    fn select(self, key: FieldKey, f: impl FnOnce(SelectBuilder) -> SelectBuilder) -> Self {
        self.push_field(f(SelectField::new(key)).into())
    }

    /// Append a code child built via a typed closure.
    #[must_use]
    fn code(self, key: FieldKey, f: impl FnOnce(CodeBuilder) -> CodeBuilder) -> Self {
        self.push_field(f(CodeField::new(key)).into())
    }

    /// Append a nested object child built via a typed closure.
    #[must_use]
    fn object(self, key: FieldKey, f: impl FnOnce(ObjectBuilder) -> ObjectBuilder) -> Self {
        let built = f(ObjectBuilder::new(key));
        self.push_field(built.into_field())
    }

    /// Append a list child built via a typed closure.
    #[must_use]
    fn list(self, key: FieldKey, f: impl FnOnce(ListBuilder) -> ListBuilder) -> Self {
        let built = f(ListBuilder::new(key));
        self.push_field(built.into_field())
    }
}
