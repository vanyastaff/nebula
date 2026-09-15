//! Typed-closure builder for [`ObjectField`].

use nebula_validator::Rule;

use crate::{
    builder::PropertyCollector,
    field::{ObjectField, Property},
    key::FieldKey,
    mode::{ExpressionMode, RequiredMode, VisibilityMode},
    widget::ObjectWidget,
};

/// Builder that produces an [`ObjectField`] with typed-closure child methods.
pub struct ObjectBuilder {
    inner: ObjectField,
}

impl ObjectBuilder {
    /// Create a new object builder bound to the given field key.
    pub fn new(key: FieldKey) -> Self {
        Self {
            inner: ObjectField::new(key),
        }
    }

    /// Set the object widget variant.
    #[must_use]
    pub fn widget(mut self, widget: ObjectWidget) -> Self {
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

    /// Mark this object field as always required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.inner = self.inner.required();
        self
    }

    /// Set required mode directly.
    #[must_use]
    pub fn required_mode(mut self, mode: RequiredMode) -> Self {
        self.inner.required = mode;
        self
    }

    /// Require this object only when the given predicate holds.
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

    /// Show this object only when the given predicate holds.
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

    /// Append an already-built property.
    #[must_use]
    pub fn property(mut self, property: impl Into<Property>) -> Self {
        self.inner = self.inner.property(property);
        self
    }

    /// Append many already-built properties at once.
    #[must_use]
    pub fn properties<I, F>(mut self, properties: I) -> Self
    where
        I: IntoIterator<Item = F>,
        F: Into<Property>,
    {
        self.inner
            .fields
            .extend(properties.into_iter().map(Into::into));
        self
    }

    /// Consume the builder and wrap the result in the top-level [`Property`] enum.
    #[must_use]
    pub fn into_property(self) -> Property {
        self.inner.into()
    }
}

impl PropertyCollector for ObjectBuilder {
    fn push_property(mut self, property: Property) -> Self {
        self.inner.fields.push(property);
        self
    }
}
