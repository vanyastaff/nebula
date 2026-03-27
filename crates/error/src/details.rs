//! TypeId-keyed extensible detail storage.
//!
//! [`ErrorDetails`] is a type-safe heterogeneous map that stores at most one
//! value per concrete type implementing [`ErrorDetail`]. This mirrors the
//! `google.rpc.Status.details` pattern where each detail type appears once.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

/// Marker trait for types that can be stored in [`ErrorDetails`].
///
/// Implement this trait on any `Send + Sync + Debug` type to make it
/// insertable into the detail map. Each concrete type occupies one slot;
/// inserting a second value of the same type overwrites the first.
///
/// # Examples
///
/// ```
/// use nebula_error::ErrorDetail;
///
/// #[derive(Debug, Clone, PartialEq, Eq)]
/// struct MyDetail {
///     message: String,
/// }
///
/// impl ErrorDetail for MyDetail {}
/// ```
pub trait ErrorDetail: Any + Send + Sync + fmt::Debug {}

/// TypeId-keyed bag of [`ErrorDetail`] values.
///
/// Stores at most one value per concrete type. Inserting a second value
/// of the same type silently overwrites the first — there is no merging.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, RetryInfo};
/// use std::time::Duration;
///
/// let mut details = ErrorDetails::new();
/// details.insert(RetryInfo {
///     retry_delay: Some(Duration::from_secs(5)),
///     max_attempts: Some(3),
/// });
///
/// assert!(details.has::<RetryInfo>());
/// let info = details.get::<RetryInfo>().unwrap();
/// assert_eq!(info.max_attempts, Some(3));
/// ```
pub struct ErrorDetails {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl ErrorDetails {
    /// Creates an empty detail map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Inserts a detail value, replacing any previous value of the same type.
    pub fn insert<T: ErrorDetail>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Returns a reference to the stored value of type `T`, if present.
    #[must_use]
    pub fn get<T: ErrorDetail>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref::<T>())
    }

    /// Returns `true` if a value of type `T` is stored.
    #[must_use]
    pub fn has<T: ErrorDetail>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Returns the number of stored detail values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if no detail values are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for ErrorDetails {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ErrorDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorDetails")
            .field("count", &self.map.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Alpha {
        value: u32,
    }
    impl ErrorDetail for Alpha {}

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Beta {
        name: String,
    }
    impl ErrorDetail for Beta {}

    #[test]
    fn insert_and_get() {
        let mut details = ErrorDetails::new();
        details.insert(Alpha { value: 42 });

        let retrieved = details.get::<Alpha>().unwrap();
        assert_eq!(retrieved.value, 42);
    }

    #[test]
    fn get_missing_returns_none() {
        let details = ErrorDetails::new();
        assert!(details.get::<Alpha>().is_none());
    }

    #[test]
    fn has_check() {
        let mut details = ErrorDetails::new();
        assert!(!details.has::<Alpha>());

        details.insert(Alpha { value: 1 });
        assert!(details.has::<Alpha>());
        assert!(!details.has::<Beta>());
    }

    #[test]
    fn multiple_types_coexist() {
        let mut details = ErrorDetails::new();
        details.insert(Alpha { value: 10 });
        details.insert(Beta {
            name: "hello".into(),
        });

        assert_eq!(details.get::<Alpha>().unwrap().value, 10);
        assert_eq!(details.get::<Beta>().unwrap().name, "hello");
        assert_eq!(details.len(), 2);
    }

    #[test]
    fn insert_overwrites_same_type() {
        let mut details = ErrorDetails::new();
        details.insert(Alpha { value: 1 });
        details.insert(Alpha { value: 2 });

        assert_eq!(details.get::<Alpha>().unwrap().value, 2);
        assert_eq!(details.len(), 1);
    }

    #[test]
    fn is_empty_and_len() {
        let mut details = ErrorDetails::new();
        assert!(details.is_empty());
        assert_eq!(details.len(), 0);

        details.insert(Alpha { value: 1 });
        assert!(!details.is_empty());
        assert_eq!(details.len(), 1);
    }
}
