//! Collection element validators

use crate::core::{ValidationError, Validator, ValidatorMetadata};
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

impl<V, T> Validator for All<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for (i, item) in input.iter().enumerate() {
            self.validator.validate(item).map_err(|e| {
                ValidationError::new("all", format!("Element at index {i} failed validation"))
                    .with_nested_error(e)
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

impl<V, T> Validator for Any<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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

impl<T> Validator for ContainsElement<T>
where
    T: PartialEq,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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

impl<T> Validator for Unique<T>
where
    T: Hash + Eq,
{
    type Input = Vec<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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
// NONE
// ============================================================================

/// Validates that no elements satisfy a condition.
#[derive(Debug, Clone)]
pub struct None<V> {
    /// The validator that no element should pass.
    pub validator: V,
}

impl<V> None<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

impl<V, T> Validator for None<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for (i, item) in input.iter().enumerate() {
            if self.validator.validate(item).is_ok() {
                return Err(ValidationError::new(
                    "none",
                    format!("Element at index {} unexpectedly passed validation", i),
                ));
            }
        }
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("None")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks that no elements satisfy a condition.
pub fn none<V>(validator: V) -> None<V> {
    None::new(validator)
}

// ============================================================================
// COUNT
// ============================================================================

/// Validates that exactly N elements satisfy a condition.
#[derive(Debug, Clone)]
pub struct Count<V> {
    /// The validator to check elements against.
    pub validator: V,
    /// The expected count of passing elements.
    pub expected: usize,
}

impl<V> Count<V> {
    pub fn new(validator: V, expected: usize) -> Self {
        Self {
            validator,
            expected,
        }
    }
}

impl<V, T> Validator for Count<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let count = input
            .iter()
            .filter(|item| self.validator.validate(item).is_ok())
            .count();

        if count == self.expected {
            Ok(())
        } else {
            Err(ValidationError::new(
                "count",
                format!(
                    "Expected {} elements to pass validation, got {}",
                    self.expected, count
                ),
            )
            .with_param("expected", self.expected.to_string())
            .with_param("actual", count.to_string()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Count")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks exactly N elements satisfy a condition.
pub fn count<V>(validator: V, expected: usize) -> Count<V> {
    Count::new(validator, expected)
}

// ============================================================================
// AT LEAST COUNT
// ============================================================================

/// Validates that at least N elements satisfy a condition.
#[derive(Debug, Clone)]
pub struct AtLeastCount<V> {
    /// The validator to check elements against.
    pub validator: V,
    /// The minimum count of passing elements.
    pub min: usize,
}

impl<V> AtLeastCount<V> {
    pub fn new(validator: V, min: usize) -> Self {
        Self { validator, min }
    }
}

impl<V, T> Validator for AtLeastCount<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let count = input
            .iter()
            .filter(|item| self.validator.validate(item).is_ok())
            .count();

        if count >= self.min {
            Ok(())
        } else {
            Err(ValidationError::new(
                "at_least_count",
                format!(
                    "Expected at least {} elements to pass validation, got {}",
                    self.min, count
                ),
            )
            .with_param("min", self.min.to_string())
            .with_param("actual", count.to_string()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("AtLeastCount")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks at least N elements satisfy a condition.
pub fn at_least_count<V>(validator: V, min: usize) -> AtLeastCount<V> {
    AtLeastCount::new(validator, min)
}

// ============================================================================
// AT MOST COUNT
// ============================================================================

/// Validates that at most N elements satisfy a condition.
#[derive(Debug, Clone)]
pub struct AtMostCount<V> {
    /// The validator to check elements against.
    pub validator: V,
    /// The maximum count of passing elements.
    pub max: usize,
}

impl<V> AtMostCount<V> {
    pub fn new(validator: V, max: usize) -> Self {
        Self { validator, max }
    }
}

impl<V, T> Validator for AtMostCount<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let count = input
            .iter()
            .filter(|item| self.validator.validate(item).is_ok())
            .count();

        if count <= self.max {
            Ok(())
        } else {
            Err(ValidationError::new(
                "at_most_count",
                format!(
                    "Expected at most {} elements to pass validation, got {}",
                    self.max, count
                ),
            )
            .with_param("max", self.max.to_string())
            .with_param("actual", count.to_string()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("AtMostCount")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks at most N elements satisfy a condition.
pub fn at_most_count<V>(validator: V, max: usize) -> AtMostCount<V> {
    AtMostCount::new(validator, max)
}

// ============================================================================
// FIRST
// ============================================================================

/// Validates the first element of a collection.
#[derive(Debug, Clone)]
pub struct First<V> {
    /// The validator to apply to the first element.
    pub validator: V,
}

impl<V> First<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

impl<V, T> Validator for First<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        match input.first() {
            Some(first) => self.validator.validate(first).map_err(|e| {
                ValidationError::new("first", "First element failed validation")
                    .with_nested_error(e)
            }),
            Option::None => Err(ValidationError::new("first", "Collection is empty")),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("First")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks the first element.
pub fn first<V>(validator: V) -> First<V> {
    First::new(validator)
}

// ============================================================================
// LAST
// ============================================================================

/// Validates the last element of a collection.
#[derive(Debug, Clone)]
pub struct Last<V> {
    /// The validator to apply to the last element.
    pub validator: V,
}

impl<V> Last<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

impl<V, T> Validator for Last<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        match input.last() {
            Some(last) => self.validator.validate(last).map_err(|e| {
                ValidationError::new("last", "Last element failed validation").with_nested_error(e)
            }),
            Option::None => Err(ValidationError::new("last", "Collection is empty")),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Last")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks the last element.
pub fn last<V>(validator: V) -> Last<V> {
    Last::new(validator)
}

// ============================================================================
// NTH
// ============================================================================

/// Validates the nth element of a collection.
#[derive(Debug, Clone)]
pub struct Nth<V> {
    /// The validator to apply to the nth element.
    pub validator: V,
    /// The index of the element to validate.
    pub index: usize,
}

impl<V> Nth<V> {
    pub fn new(validator: V, index: usize) -> Self {
        Self { validator, index }
    }
}

impl<V, T> Validator for Nth<V>
where
    V: Validator<Input = T>,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        match input.get(self.index) {
            Some(item) => self.validator.validate(item).map_err(|e| {
                ValidationError::new(
                    "nth",
                    format!("Element at index {} failed validation", self.index),
                )
                .with_nested_error(e)
            }),
            Option::None => Err(ValidationError::new(
                "nth",
                format!(
                    "Index {} out of bounds for collection of size {}",
                    self.index,
                    input.len()
                ),
            )),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Nth")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks the nth element.
pub fn nth<V>(validator: V, index: usize) -> Nth<V> {
    Nth::new(validator, index)
}

// ============================================================================
// SORTED
// ============================================================================

/// Validates that a collection is sorted in ascending order.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sorted<T> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Validator for Sorted<T>
where
    T: PartialOrd,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for window in input.windows(2) {
            if window[0] > window[1] {
                return Err(ValidationError::new(
                    "sorted",
                    "Collection must be sorted in ascending order",
                ));
            }
        }
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Sorted")
            .with_tag("collection")
            .with_tag("order")
    }
}

/// Creates a validator that checks if a collection is sorted ascending.
#[must_use]
pub fn sorted<T>() -> Sorted<T>
where
    T: PartialOrd,
{
    Sorted {
        _phantom: std::marker::PhantomData,
    }
}

// ============================================================================
// SORTED DESCENDING
// ============================================================================

/// Validates that a collection is sorted in descending order.
#[derive(Debug, Clone, Copy, Default)]
pub struct SortedDescending<T> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Validator for SortedDescending<T>
where
    T: PartialOrd,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for window in input.windows(2) {
            if window[0] < window[1] {
                return Err(ValidationError::new(
                    "sorted_descending",
                    "Collection must be sorted in descending order",
                ));
            }
        }
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("SortedDescending")
            .with_tag("collection")
            .with_tag("order")
    }
}

/// Creates a validator that checks if a collection is sorted descending.
#[must_use]
pub fn sorted_descending<T>() -> SortedDescending<T>
where
    T: PartialOrd,
{
    SortedDescending {
        _phantom: std::marker::PhantomData,
    }
}

// ============================================================================
// CONTAINS ALL
// ============================================================================

/// Validates that a collection contains all specified elements.
#[derive(Debug, Clone)]
pub struct ContainsAll<T> {
    /// The elements that must all be present.
    pub elements: Vec<T>,
}

impl<T> ContainsAll<T> {
    pub fn new(elements: Vec<T>) -> Self {
        Self { elements }
    }
}

impl<T> Validator for ContainsAll<T>
where
    T: PartialEq,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for element in &self.elements {
            if !input.contains(element) {
                return Err(ValidationError::new(
                    "contains_all",
                    "Collection must contain all specified elements",
                ));
            }
        }
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("ContainsAll")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks if a collection contains all specified elements.
pub fn contains_all<T>(elements: Vec<T>) -> ContainsAll<T>
where
    T: PartialEq,
{
    ContainsAll::new(elements)
}

// ============================================================================
// CONTAINS ANY
// ============================================================================

/// Validates that a collection contains at least one of the specified elements.
#[derive(Debug, Clone)]
pub struct ContainsAny<T> {
    /// The elements, at least one of which must be present.
    pub elements: Vec<T>,
}

impl<T> ContainsAny<T> {
    pub fn new(elements: Vec<T>) -> Self {
        Self { elements }
    }
}

impl<T> Validator for ContainsAny<T>
where
    T: PartialEq,
{
    type Input = [T];

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for element in &self.elements {
            if input.contains(element) {
                return Ok(());
            }
        }
        Err(ValidationError::new(
            "contains_any",
            "Collection must contain at least one of the specified elements",
        ))
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("ContainsAny")
            .with_tag("collection")
            .with_tag("elements")
    }
}

/// Creates a validator that checks if a collection contains any of the specified elements.
pub fn contains_any<T>(elements: Vec<T>) -> ContainsAny<T>
where
    T: PartialEq,
{
    ContainsAny::new(elements)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validators::numeric::positive;

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

    #[test]
    fn test_none() {
        let validator = none(positive::<i32>());
        assert!(validator.validate(&[-1, -2, -3]).is_ok());
        assert!(validator.validate(&[-1, 2, -3]).is_err());
    }

    #[test]
    fn test_count() {
        let validator = count(positive::<i32>(), 2);
        assert!(validator.validate(&[1, -1, 2, -2]).is_ok());
        assert!(validator.validate(&[1, 2, 3]).is_err()); // 3 positive, expected 2
        assert!(validator.validate(&[1]).is_err()); // 1 positive, expected 2
    }

    #[test]
    fn test_at_least_count() {
        let validator = at_least_count(positive::<i32>(), 2);
        assert!(validator.validate(&[1, 2]).is_ok());
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[1, -1]).is_err());
    }

    #[test]
    fn test_at_most_count() {
        let validator = at_most_count(positive::<i32>(), 2);
        assert!(validator.validate(&[1, 2]).is_ok());
        assert!(validator.validate(&[1, -1]).is_ok());
        assert!(validator.validate(&[1, 2, 3]).is_err());
    }

    #[test]
    fn test_first() {
        let validator = first(positive::<i32>());
        assert!(validator.validate(&[1, -2, -3]).is_ok());
        assert!(validator.validate(&[-1, 2, 3]).is_err());
        assert!(validator.validate(&([] as [i32; 0])).is_err()); // empty
    }

    #[test]
    fn test_last() {
        let validator = last(positive::<i32>());
        assert!(validator.validate(&[-1, -2, 3]).is_ok());
        assert!(validator.validate(&[1, 2, -3]).is_err());
        assert!(validator.validate(&([] as [i32; 0])).is_err()); // empty
    }

    #[test]
    fn test_nth() {
        let validator = nth(positive::<i32>(), 1);
        assert!(validator.validate(&[-1, 2, -3]).is_ok());
        assert!(validator.validate(&[1, -2, 3]).is_err());
        assert!(validator.validate(&[1]).is_err()); // index out of bounds
    }

    #[test]
    fn test_sorted() {
        let validator = sorted::<i32>();
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[1, 1, 2]).is_ok());
        assert!(validator.validate(&([] as [i32; 0])).is_ok());
        assert!(validator.validate(&[3, 2, 1]).is_err());
    }

    #[test]
    fn test_sorted_descending() {
        let validator = sorted_descending::<i32>();
        assert!(validator.validate(&[3, 2, 1]).is_ok());
        assert!(validator.validate(&[3, 3, 2]).is_ok());
        assert!(validator.validate(&[1, 2, 3]).is_err());
    }

    #[test]
    fn test_contains_all() {
        let validator = contains_all(vec![1, 2]);
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[1, 2]).is_ok());
        assert!(validator.validate(&[1, 3]).is_err());
    }

    #[test]
    fn test_contains_any() {
        let validator = contains_any(vec![1, 2]);
        assert!(validator.validate(&[1, 3, 4]).is_ok());
        assert!(validator.validate(&[2, 3, 4]).is_ok());
        assert!(validator.validate(&[3, 4, 5]).is_err());
    }
}
