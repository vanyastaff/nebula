//! Batch/validation error aggregation.
//!
//! [`ErrorCollection`] groups multiple [`NebulaError`] values for scenarios
//! where a single operation can produce several independent failures — e.g.
//! form validation, batch processing, or parallel execution.

use crate::error::NebulaError;
use crate::traits::Classify;
use crate::{ErrorCategory, ErrorSeverity};

/// A collection of [`NebulaError`] values for batch/validation scenarios.
///
/// Provides aggregate queries like [`any_retryable`](Self::any_retryable),
/// [`max_severity`](Self::max_severity), and
/// [`uniform_category`](Self::uniform_category) to help callers decide
/// how to handle the batch as a whole.
///
/// # Examples
///
/// ```
/// use nebula_error::{
///     Classify, ErrorCategory, ErrorCode, ErrorSeverity,
///     ErrorCollection, NebulaError, codes,
/// };
///
/// #[derive(Debug)]
/// struct ValErr(String);
/// impl std::fmt::Display for ValErr {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         f.write_str(&self.0)
///     }
/// }
/// impl Classify for ValErr {
///     fn category(&self) -> ErrorCategory { ErrorCategory::Validation }
///     fn code(&self) -> ErrorCode { codes::VALIDATION.clone() }
/// }
///
/// let mut coll = ErrorCollection::new();
/// coll.push(NebulaError::new(ValErr("bad email".into())));
/// coll.push(NebulaError::new(ValErr("bad phone".into())));
///
/// assert_eq!(coll.len(), 2);
/// assert!(!coll.any_retryable());
/// assert_eq!(coll.uniform_category(), Some(ErrorCategory::Validation));
/// ```
pub struct ErrorCollection<E: Classify> {
    errors: Vec<NebulaError<E>>,
}

impl<E: Classify> ErrorCollection<E> {
    /// Creates an empty collection.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{Classify, ErrorCategory, ErrorCode, ErrorCollection, codes};
    ///
    /// # #[derive(Debug)]
    /// # struct E;
    /// # impl std::fmt::Display for E {
    /// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("e") }
    /// # }
    /// # impl Classify for E {
    /// #     fn category(&self) -> ErrorCategory { ErrorCategory::Internal }
    /// #     fn code(&self) -> ErrorCode { codes::INTERNAL.clone() }
    /// # }
    /// let coll: ErrorCollection<E> = ErrorCollection::new();
    /// assert!(coll.is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Appends an error to the collection.
    pub fn push(&mut self, error: NebulaError<E>) {
        self.errors.push(error);
    }

    /// Returns `true` if the collection contains no errors.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns the number of errors in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Returns an iterator over references to the errors.
    pub fn iter(&self) -> std::slice::Iter<'_, NebulaError<E>> {
        self.errors.iter()
    }

    /// Returns `true` if any error in the collection is retryable.
    ///
    /// Returns `false` for an empty collection.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{
    ///     Classify, ErrorCategory, ErrorCode, ErrorCollection, NebulaError, codes,
    /// };
    ///
    /// # #[derive(Debug)]
    /// # struct TimeoutErr;
    /// # impl std::fmt::Display for TimeoutErr {
    /// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    /// #         f.write_str("timeout")
    /// #     }
    /// # }
    /// # impl Classify for TimeoutErr {
    /// #     fn category(&self) -> ErrorCategory { ErrorCategory::Timeout }
    /// #     fn code(&self) -> ErrorCode { codes::TIMEOUT.clone() }
    /// # }
    /// let mut coll = ErrorCollection::new();
    /// coll.push(NebulaError::new(TimeoutErr));
    /// assert!(coll.any_retryable());
    /// ```
    #[must_use]
    pub fn any_retryable(&self) -> bool {
        self.errors.iter().any(|e| e.is_retryable())
    }

    /// Returns the highest severity among all errors.
    ///
    /// Returns [`ErrorSeverity::Info`] for an empty collection.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{
    ///     Classify, ErrorCategory, ErrorCode, ErrorCollection,
    ///     ErrorSeverity, NebulaError, codes,
    /// };
    ///
    /// # #[derive(Debug)]
    /// # struct E;
    /// # impl std::fmt::Display for E {
    /// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("e") }
    /// # }
    /// # impl Classify for E {
    /// #     fn category(&self) -> ErrorCategory { ErrorCategory::Internal }
    /// #     fn code(&self) -> ErrorCode { codes::INTERNAL.clone() }
    /// # }
    /// let coll: ErrorCollection<E> = ErrorCollection::new();
    /// assert_eq!(coll.max_severity(), ErrorSeverity::Info);
    /// ```
    #[must_use]
    pub fn max_severity(&self) -> ErrorSeverity {
        self.errors
            .iter()
            .map(|e| e.severity())
            .max()
            .unwrap_or(ErrorSeverity::Info)
    }

    /// If all errors share the same category, returns it. Otherwise
    /// returns `None`.
    ///
    /// Returns `None` for an empty collection.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{
    ///     Classify, ErrorCategory, ErrorCode, ErrorCollection, NebulaError, codes,
    /// };
    ///
    /// # #[derive(Debug)]
    /// # struct ValErr;
    /// # impl std::fmt::Display for ValErr {
    /// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    /// #         f.write_str("val")
    /// #     }
    /// # }
    /// # impl Classify for ValErr {
    /// #     fn category(&self) -> ErrorCategory { ErrorCategory::Validation }
    /// #     fn code(&self) -> ErrorCode { codes::VALIDATION.clone() }
    /// # }
    /// let mut coll = ErrorCollection::new();
    /// coll.push(NebulaError::new(ValErr));
    /// coll.push(NebulaError::new(ValErr));
    /// assert_eq!(coll.uniform_category(), Some(ErrorCategory::Validation));
    /// ```
    #[must_use]
    pub fn uniform_category(&self) -> Option<ErrorCategory> {
        let mut iter = self.errors.iter();
        let first = iter.next()?.category();
        if iter.all(|e| e.category() == first) {
            Some(first)
        } else {
            None
        }
    }
}

impl<E: Classify> Default for ErrorCollection<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Classify + std::fmt::Debug> std::fmt::Debug for ErrorCollection<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErrorCollection")
            .field("len", &self.errors.len())
            .field("errors", &self.errors)
            .finish()
    }
}

impl<E: Classify> IntoIterator for ErrorCollection<E> {
    type Item = NebulaError<E>;
    type IntoIter = std::vec::IntoIter<NebulaError<E>>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.into_iter()
    }
}

impl<'a, E: Classify> IntoIterator for &'a ErrorCollection<E> {
    type Item = &'a NebulaError<E>;
    type IntoIter = std::slice::Iter<'a, NebulaError<E>>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.iter()
    }
}

impl<E: Classify> FromIterator<NebulaError<E>> for ErrorCollection<E> {
    fn from_iter<I: IntoIterator<Item = NebulaError<E>>>(iter: I) -> Self {
        Self {
            errors: iter.into_iter().collect(),
        }
    }
}

/// Result type for batch operations that may partially succeed.
///
/// On success, contains the value `T`. On failure, contains an
/// [`ErrorCollection`] with all accumulated errors.
///
/// # Examples
///
/// ```
/// use nebula_error::{
///     BatchResult, Classify, ErrorCategory, ErrorCode,
///     ErrorCollection, NebulaError, codes,
/// };
///
/// # #[derive(Debug)]
/// # struct E;
/// # impl std::fmt::Display for E {
/// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("e") }
/// # }
/// # impl Classify for E {
/// #     fn category(&self) -> ErrorCategory { ErrorCategory::Validation }
/// #     fn code(&self) -> ErrorCode { codes::VALIDATION.clone() }
/// # }
/// fn validate() -> BatchResult<(), E> {
///     let mut errors = ErrorCollection::new();
///     errors.push(NebulaError::new(E));
///     Err(errors)
/// }
///
/// assert!(validate().is_err());
/// ```
pub type BatchResult<T, E> = std::result::Result<T, ErrorCollection<E>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes;
    use std::fmt;

    #[derive(Debug, Clone)]
    struct TestErr {
        cat: ErrorCategory,
        sev: ErrorSeverity,
    }

    impl fmt::Display for TestErr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "test({})", self.cat)
        }
    }

    impl Classify for TestErr {
        fn category(&self) -> ErrorCategory {
            self.cat
        }
        fn code(&self) -> crate::ErrorCode {
            codes::INTERNAL.clone()
        }
        fn severity(&self) -> ErrorSeverity {
            self.sev
        }
    }

    fn val_error() -> TestErr {
        TestErr {
            cat: ErrorCategory::Validation,
            sev: ErrorSeverity::Error,
        }
    }

    fn timeout_error() -> TestErr {
        TestErr {
            cat: ErrorCategory::Timeout,
            sev: ErrorSeverity::Warning,
        }
    }

    #[test]
    fn empty_collection() {
        let coll: ErrorCollection<TestErr> = ErrorCollection::new();
        assert!(coll.is_empty());
        assert_eq!(coll.len(), 0);
        assert!(!coll.any_retryable());
        assert_eq!(coll.max_severity(), ErrorSeverity::Info);
        assert_eq!(coll.uniform_category(), None);
    }

    #[test]
    fn push_and_iterate() {
        let mut coll = ErrorCollection::new();
        coll.push(NebulaError::new(val_error()));
        coll.push(NebulaError::new(val_error()));

        assert_eq!(coll.len(), 2);
        assert!(!coll.is_empty());

        let categories: Vec<_> = coll.iter().map(|e| e.category()).collect();
        assert_eq!(
            categories,
            vec![ErrorCategory::Validation, ErrorCategory::Validation]
        );
    }

    #[test]
    fn any_retryable() {
        let mut coll = ErrorCollection::new();
        coll.push(NebulaError::new(val_error()));
        assert!(!coll.any_retryable());

        coll.push(NebulaError::new(timeout_error()));
        assert!(coll.any_retryable());
    }

    #[test]
    fn max_severity_empty_and_non_empty() {
        let empty: ErrorCollection<TestErr> = ErrorCollection::new();
        assert_eq!(empty.max_severity(), ErrorSeverity::Info);

        let mut coll = ErrorCollection::new();
        coll.push(NebulaError::new(timeout_error())); // Warning
        assert_eq!(coll.max_severity(), ErrorSeverity::Warning);

        coll.push(NebulaError::new(val_error())); // Error
        assert_eq!(coll.max_severity(), ErrorSeverity::Error);
    }

    #[test]
    fn uniform_category_same() {
        let mut coll = ErrorCollection::new();
        coll.push(NebulaError::new(val_error()));
        coll.push(NebulaError::new(val_error()));
        assert_eq!(coll.uniform_category(), Some(ErrorCategory::Validation));
    }

    #[test]
    fn uniform_category_mixed() {
        let mut coll = ErrorCollection::new();
        coll.push(NebulaError::new(val_error()));
        coll.push(NebulaError::new(timeout_error()));
        assert_eq!(coll.uniform_category(), None);
    }

    #[test]
    fn into_iterator() {
        let mut coll = ErrorCollection::new();
        coll.push(NebulaError::new(val_error()));
        coll.push(NebulaError::new(timeout_error()));

        let collected: Vec<_> = coll.into_iter().map(|e| e.category()).collect();
        assert_eq!(
            collected,
            vec![ErrorCategory::Validation, ErrorCategory::Timeout]
        );
    }

    #[test]
    fn from_iterator() {
        let errors = vec![NebulaError::new(val_error()), NebulaError::new(val_error())];
        let coll: ErrorCollection<TestErr> = errors.into_iter().collect();
        assert_eq!(coll.len(), 2);
    }

    #[test]
    fn default_is_empty() {
        let coll: ErrorCollection<TestErr> = ErrorCollection::default();
        assert!(coll.is_empty());
    }
}
