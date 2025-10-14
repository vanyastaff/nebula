//! Collection element validators

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata};
use std::collections::HashSet;
use std::hash::Hash;

// ============================================================================
// ALL
// ============================================================================

/// Validates that all elements satisfy a condition.
#[derive(Debug, Clone)]
pub struct All<V> {
    /// The validator applied to all elements in the collection.
    pub validator: V,
}

impl<V> All<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

impl<V, T> TypedValidator for All<V>
where
    V: TypedValidator<Input = T>,
    V::Error: Into<ValidationError>,
{
    type Input = [T];
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        for (i, item) in input.iter().enumerate() {
            self.validator.validate(item).map_err(|e| {
                ValidationError::new("all", format!("Element at index {i} failed validation"))
                    .with_nested_error(e.into())
            })?;
        }
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("All")
            .with_tag("collection")
            .with_tag("elements")
    }
}

pub fn all<V>(validator: V) -> All<V> {
    All::new(validator)
}

// ============================================================================
// ANY
// ============================================================================

/// Validates that at least one element satisfies a condition.
#[derive(Debug, Clone)]
pub struct Any<V> {
    /// The validator checked against all elements (at least one must pass).
    pub validator: V,
}

impl<V> Any<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

impl<V, T> TypedValidator for Any<V>
where
    V: TypedValidator<Input = T>,
{
    type Input = [T];
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        for item in input {
            if self.validator.validate(item).is_ok() {
                return Ok(());
            }
        }
        Err(ValidationError::new(
            "any",
            "At least one element must satisfy the condition",
        ))
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Any")
            .with_tag("collection")
            .with_tag("elements")
    }
}

pub fn any<V>(validator: V) -> Any<V> {
    Any::new(validator)
}

// ============================================================================
// CONTAINS ELEMENT
// ============================================================================

/// Validates that collection contains a specific element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainsElement<T> {
    /// The element to search for in the collection.
    pub element: T,
}

impl<T> ContainsElement<T> {
    pub fn new(element: T) -> Self {
        Self { element }
    }
}

impl<T> TypedValidator for ContainsElement<T>
where
    T: PartialEq,
{
    type Input = [T];
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.contains(&self.element) {
            Ok(())
        } else {
            Err(ValidationError::new(
                "contains",
                "Collection must contain the element",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("ContainsElement")
            .with_tag("collection")
            .with_tag("elements")
    }
}

pub fn contains_element<T>(element: T) -> ContainsElement<T> {
    ContainsElement::new(element)
}

// ============================================================================
// UNIQUE
// ============================================================================

/// Validates that all elements are unique.
#[derive(Debug, Clone, Copy)]
pub struct Unique<T> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T> TypedValidator for Unique<T>
where
    T: Hash + Eq,
{
    type Input = Vec<T>;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let mut seen = HashSet::new();
        for item in input {
            if !seen.insert(item) {
                return Err(ValidationError::new(
                    "unique",
                    "All elements must be unique",
                ));
            }
        }
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Unique")
            .with_tag("collection")
            .with_tag("elements")
    }
}

#[must_use]
pub fn unique<T>() -> Unique<T>
where
    T: Hash + Eq,
{
    Unique {
        _phantom: std::marker::PhantomData,
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unique() {
        let validator = unique();
        assert!(validator.validate(&vec![1, 2, 3]).is_ok());
        assert!(validator.validate(&vec![1, 2, 2]).is_err());
    }

    #[test]
    fn test_contains_element() {
        let validator = contains_element(2);
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[1, 3]).is_err());
    }
}
