//! JSON field combinator for validating fields within `serde_json::Value`.
//!
//! Uses RFC 6901 JSON Pointer syntax for path traversal via
//! `serde_json::Value::pointer()`.

use crate::foundation::validatable::AsValidatable;
use crate::foundation::{Validate, ValidationError};
use std::borrow::{Borrow, Cow};
use std::fmt;
use std::marker::PhantomData;

/// Validates a field within a JSON value by RFC 6901 JSON Pointer path.
///
/// Extracts a value at the given pointer path and validates it with the
/// inner validator. Supports required (default) and optional modes.
///
/// # Type Parameters
///
/// * `V` - The inner validator type
/// * `I` - The target type that JSON values are converted to (e.g., `str`, `i64`)
///
/// # Path syntax
///
/// Uses [RFC 6901 JSON Pointer](https://www.rfc-editor.org/rfc/rfc6901):
/// - `"/server/port"` — nested object access
/// - `"/items/0/name"` — array index + nested key
/// - `""` — root value
pub struct JsonField<V, I: ?Sized> {
    pointer: Cow<'static, str>,
    inner: V,
    required: bool,
    _phantom: PhantomData<fn(&I)>,
}

impl<V, I: ?Sized> JsonField<V, I> {
    /// Creates a required field validator.
    ///
    /// Validation fails if the path does not exist in the input.
    pub fn required(pointer: impl Into<Cow<'static, str>>, inner: V) -> Self {
        Self {
            pointer: pointer.into(),
            inner,
            required: true,
            _phantom: PhantomData,
        }
    }

    /// Creates an optional field validator.
    ///
    /// Missing paths and `null` values pass validation silently.
    pub fn optional(pointer: impl Into<Cow<'static, str>>, inner: V) -> Self {
        Self {
            pointer: pointer.into(),
            inner,
            required: false,
            _phantom: PhantomData,
        }
    }
}

impl<V, I> Validate<serde_json::Value> for JsonField<V, I>
where
    V: Validate<I>,
    I: ?Sized,
    serde_json::Value: AsValidatable<I>,
{
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        let resolved = if self.pointer.is_empty() {
            Some(input)
        } else {
            input.pointer(&self.pointer)
        };

        match resolved {
            Some(value) if !self.required && value.is_null() => Ok(()),
            Some(value) => {
                let converted = AsValidatable::<I>::as_validatable(value)
                    .map_err(|e| e.with_field(self.pointer.clone()))?;
                self.inner
                    .validate(converted.borrow())
                    .map_err(|e| e.with_field(self.pointer.clone()))
            }
            None if !self.required => Ok(()),
            None => Err(ValidationError::new(
                "path_not_found",
                format!("Path '{}' not found", self.pointer),
            )
            .with_field(self.pointer.clone())
            .with_param("path", self.pointer.clone())),
        }
    }
}

impl<V: fmt::Debug, I: ?Sized> fmt::Debug for JsonField<V, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonField")
            .field("pointer", &self.pointer)
            .field("inner", &self.inner)
            .field("required", &self.required)
            .finish()
    }
}

impl<V: Clone, I: ?Sized> Clone for JsonField<V, I> {
    fn clone(&self) -> Self {
        Self {
            pointer: self.pointer.clone(),
            inner: self.inner.clone(),
            required: self.required,
            _phantom: PhantomData,
        }
    }
}

/// Creates a required JSON field validator.
///
/// # Examples
///
/// ```
/// use nebula_validator::combinators::json_field;
/// use nebula_validator::validators::min;
/// use nebula_validator::foundation::Validate;
/// use serde_json::json;
///
/// let v = json_field("/port", min::<i64>(1));
/// assert!(v.validate(&json!({"port": 8080})).is_ok());
/// assert!(v.validate(&json!({"port": 0})).is_err());
/// ```
pub fn json_field<V, I: ?Sized>(
    pointer: impl Into<Cow<'static, str>>,
    validator: V,
) -> JsonField<V, I> {
    JsonField::required(pointer, validator)
}

/// Creates an optional JSON field validator.
///
/// Missing paths and `null` values pass validation silently.
///
/// # Examples
///
/// ```
/// use nebula_validator::combinators::json_field_optional;
/// use nebula_validator::validators::min_length;
/// use nebula_validator::foundation::Validate;
/// use serde_json::json;
///
/// let v = json_field_optional("/email", min_length(5));
/// assert!(v.validate(&json!({"name": "Alice"})).is_ok()); // missing = ok
/// assert!(v.validate(&json!({"email": null})).is_ok());    // null = ok
/// assert!(v.validate(&json!({"email": "a@b.c"})).is_ok()); // valid = ok
/// ```
pub fn json_field_optional<V, I: ?Sized>(
    pointer: impl Into<Cow<'static, str>>,
    validator: V,
) -> JsonField<V, I> {
    JsonField::optional(pointer, validator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::ValidateExt;
    use crate::validators::min_length;
    use serde_json::json;

    #[test]
    fn required_field_valid() {
        let v = json_field("/name", min_length(3));
        assert!(v.validate(&json!({"name": "Alice"})).is_ok());
    }

    #[test]
    fn required_field_invalid() {
        let v = json_field("/name", min_length(3));
        let err = v.validate(&json!({"name": "Al"})).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("/name"));
    }

    #[test]
    fn required_field_missing() {
        let v = json_field("/name", min_length(3));
        let err = v.validate(&json!({"age": 25})).unwrap_err();
        assert_eq!(err.code.as_ref(), "path_not_found");
    }

    #[test]
    fn optional_field_missing_passes() {
        let v = json_field_optional("/name", min_length(3));
        assert!(v.validate(&json!({"age": 25})).is_ok());
    }

    #[test]
    fn optional_field_null_passes() {
        let v = json_field_optional("/name", min_length(3));
        assert!(v.validate(&json!({"name": null})).is_ok());
    }

    #[test]
    fn optional_field_present_invalid() {
        let v = json_field_optional("/name", min_length(3));
        assert!(v.validate(&json!({"name": "Al"})).is_err());
    }

    #[test]
    fn nested_path() {
        let v = json_field("/server/host", min_length(1));
        assert!(
            v.validate(&json!({"server": {"host": "localhost"}}))
                .is_ok()
        );
    }

    #[test]
    fn array_index_path() {
        let v = json_field("/tags/0", min_length(1));
        assert!(v.validate(&json!({"tags": ["web", "api"]})).is_ok());
    }

    #[test]
    fn type_mismatch_error() {
        let v = json_field("/name", min_length(1));
        let err = v.validate(&json!({"name": 42})).unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
        assert_eq!(err.field.as_deref(), Some("/name"));
    }

    #[test]
    fn composition_and() {
        let v = json_field("/first", min_length(1)).and(json_field("/last", min_length(1)));
        assert!(
            v.validate(&json!({"first": "Alice", "last": "Smith"}))
                .is_ok()
        );
    }

    #[test]
    fn root_pointer() {
        let v = json_field("", min_length(3));
        assert!(v.validate(&json!("hello")).is_ok());
        assert!(v.validate(&json!("hi")).is_err());
    }
}
