//! [`ValidationErrors`] — collection type for accumulating multiple errors.

use std::{borrow::Cow, fmt};

use super::validation_error::ValidationError;

/// A collection of validation errors.
///
/// Useful for collecting multiple validation errors before returning them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationErrors {
    errors: Vec<ValidationError>,
}

impl ValidationErrors {
    /// Creates a new empty error collection.
    #[must_use]
    #[inline]
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Adds an error to the collection.
    #[inline]
    pub fn add(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    /// Adds multiple errors to the collection.
    #[inline]
    pub fn extend(&mut self, errors: impl IntoIterator<Item = ValidationError>) {
        self.errors.extend(errors);
    }

    /// Returns true if there are any errors.
    #[must_use]
    #[inline]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Returns the number of errors.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Returns true if empty.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns all errors.
    #[must_use]
    #[inline]
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Returns a mutable reference to the last error, if any.
    #[inline]
    pub fn last_mut(&mut self) -> Option<&mut ValidationError> {
        self.errors.last_mut()
    }

    /// Converts to a single error with nested errors.
    #[inline]
    pub fn into_single_error(self, message: impl Into<Cow<'static, str>>) -> ValidationError {
        ValidationError::new("validation_errors", message).with_nested(self.errors)
    }

    /// Converts to a Result.
    #[must_use = "result must be used"]
    #[inline]
    pub fn into_result<T>(self, ok_value: T) -> Result<T, ValidationErrors> {
        if self.is_empty() {
            Ok(ok_value)
        } else {
            Err(self)
        }
    }
}

impl FromIterator<ValidationError> for ValidationErrors {
    fn from_iter<I: IntoIterator<Item = ValidationError>>(iter: I) -> Self {
        Self {
            errors: iter.into_iter().collect(),
        }
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Validation failed with {} error(s):", self.errors.len())?;
        for (i, error) in self.errors.iter().enumerate() {
            writeln!(f, "  {}. {}", i + 1, error)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

impl IntoIterator for ValidationErrors {
    type Item = ValidationError;
    type IntoIter = std::vec::IntoIter<ValidationError>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.into_iter()
    }
}

impl<'a> IntoIterator for &'a ValidationErrors {
    type Item = &'a ValidationError;
    type IntoIter = std::slice::Iter<'a, ValidationError>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.iter()
    }
}
