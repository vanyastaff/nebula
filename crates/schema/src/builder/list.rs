//! Typed-closure builder for [`ListField`](crate::field::ListField).

use nebula_validator::Rule;

use crate::{
    builder::ObjectBuilder,
    field::{
        BooleanField, CodeField, Field, ListField, NumberField, SecretField, SelectField,
        StringField,
    },
    key::FieldKey,
    mode::{ExpressionMode, VisibilityMode},
    widget::ListWidget,
};

/// Builder that produces a [`ListField`] with typed-closure item methods.
///
/// Unlike [`ObjectBuilder`], `ListBuilder` holds a single item schema — not a
/// collection of children — so its `item_*` methods replace the previous item
/// rather than appending.
pub struct ListBuilder {
    inner: ListField,
}

impl ListBuilder {
    /// Create a new list builder bound to the given field key.
    pub fn new(key: FieldKey) -> Self {
        Self {
            inner: ListField::new(key),
        }
    }

    /// Set the list widget variant.
    #[must_use]
    pub fn widget(mut self, widget: ListWidget) -> Self {
        self.inner = self.inner.widget(widget);
        self
    }

    /// Set a human-readable label.
    #[must_use]
    pub fn label(mut self, value: impl Into<String>) -> Self {
        self.inner = self.inner.label(value);
        self
    }

    /// Set a help description.
    #[must_use]
    pub fn description(mut self, value: impl Into<String>) -> Self {
        self.inner = self.inner.description(value);
        self
    }

    /// Mark this list field as always required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.inner = self.inner.required();
        self
    }

    /// Require this list only when the given predicate holds.
    #[must_use]
    pub fn required_when(mut self, rule: Rule) -> Self {
        self.inner = self.inner.required_when(rule);
        self
    }

    /// Set the visibility mode directly.
    #[must_use]
    pub fn visible(mut self, mode: VisibilityMode) -> Self {
        self.inner = self.inner.visible(mode);
        self
    }

    /// Show this list only when the given predicate holds.
    #[must_use]
    pub fn visible_when(mut self, rule: Rule) -> Self {
        self.inner = self.inner.visible_when(rule);
        self
    }

    /// Set the expression mode.
    #[must_use]
    pub fn expression_mode(mut self, mode: ExpressionMode) -> Self {
        self.inner = self.inner.expression_mode(mode);
        self
    }

    /// Forbid expression values on this field.
    #[must_use]
    pub fn no_expression(mut self) -> Self {
        self.inner = self.inner.no_expression();
        self
    }

    /// Set minimum item count.
    #[must_use]
    pub fn min_items(mut self, count: u32) -> Self {
        self.inner = self.inner.min_items(count);
        self
    }

    /// Set maximum item count.
    #[must_use]
    pub fn max_items(mut self, count: u32) -> Self {
        self.inner = self.inner.max_items(count);
        self
    }

    /// Require items to be unique.
    #[must_use]
    pub fn unique(mut self) -> Self {
        self.inner = self.inner.unique();
        self
    }

    /// Set the item schema from an already-built field.
    #[must_use]
    pub fn item(mut self, item: impl Into<Field>) -> Self {
        self.inner = self.inner.item(item);
        self
    }

    /// Set a string item schema via a typed closure.
    #[must_use]
    pub fn item_string(
        mut self,
        key: FieldKey,
        f: impl FnOnce(StringField) -> StringField,
    ) -> Self {
        let built = f(StringField::new(key));
        self.inner = self.inner.item(built);
        self
    }

    /// Set a number item schema via a typed closure.
    #[must_use]
    pub fn item_number(
        mut self,
        key: FieldKey,
        f: impl FnOnce(NumberField) -> NumberField,
    ) -> Self {
        let built = f(NumberField::new(key));
        self.inner = self.inner.item(built);
        self
    }

    /// Set a boolean item schema via a typed closure.
    #[must_use]
    pub fn item_boolean(
        mut self,
        key: FieldKey,
        f: impl FnOnce(BooleanField) -> BooleanField,
    ) -> Self {
        let built = f(BooleanField::new(key));
        self.inner = self.inner.item(built);
        self
    }

    /// Set a select item schema via a typed closure.
    #[must_use]
    pub fn item_select(
        mut self,
        key: FieldKey,
        f: impl FnOnce(SelectField) -> SelectField,
    ) -> Self {
        let built = f(SelectField::new(key));
        self.inner = self.inner.item(built);
        self
    }

    /// Set a code item schema via a typed closure.
    #[must_use]
    pub fn item_code(mut self, key: FieldKey, f: impl FnOnce(CodeField) -> CodeField) -> Self {
        let built = f(CodeField::new(key));
        self.inner = self.inner.item(built);
        self
    }

    /// Set a secret item schema via a typed closure.
    #[must_use]
    pub fn item_secret(
        mut self,
        key: FieldKey,
        f: impl FnOnce(SecretField) -> SecretField,
    ) -> Self {
        let built = f(SecretField::new(key));
        self.inner = self.inner.item(built);
        self
    }

    /// Set a nested object item schema via a typed closure.
    #[must_use]
    pub fn item_object(
        mut self,
        key: FieldKey,
        f: impl FnOnce(ObjectBuilder) -> ObjectBuilder,
    ) -> Self {
        let built = f(ObjectBuilder::new(key));
        self.inner = self.inner.item(built.into_field());
        self
    }

    /// Consume the builder and wrap the result in the top-level [`Field`] enum.
    #[must_use]
    pub fn into_field(self) -> Field {
        self.inner.into()
    }
}
