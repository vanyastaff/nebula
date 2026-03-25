//! Paginated result wrapper for loader responses.
//!
//! [`LoaderResult`] provides a uniform envelope for all loader return values,
//! supporting optional cursor-based pagination and total-count metadata.

use serde::{Deserialize, Serialize};

/// Paginated result returned by loader functions.
///
/// Wraps a page of items with optional cursor-based pagination metadata.
/// Loaders that return all results at once can use [`LoaderResult::done`];
/// those that paginate use [`LoaderResult::page`].
///
/// # Examples
///
/// ```
/// use nebula_parameter::loader_result::LoaderResult;
///
/// // All results in one shot
/// let result = LoaderResult::done(vec!["a", "b", "c"]);
/// assert!(!result.has_more());
///
/// // Paginated response
/// let result = LoaderResult::page(vec!["a", "b"], "cursor_abc")
///     .with_total(100);
/// assert!(result.has_more());
/// assert_eq!(result.total, Some(100));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderResult<T> {
    /// The items in this page of results.
    pub items: Vec<T>,
    /// Opaque cursor for fetching the next page, if more results exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Optional total count of all items across all pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl<T> LoaderResult<T> {
    /// Creates a complete (non-paginated) result containing all items.
    pub fn done(items: Vec<T>) -> Self {
        Self {
            items,
            next_cursor: None,
            total: None,
        }
    }

    /// Creates a paginated result with a cursor pointing to the next page.
    pub fn page(items: Vec<T>, cursor: impl Into<String>) -> Self {
        Self {
            items,
            next_cursor: Some(cursor.into()),
            total: None,
        }
    }

    /// Attaches a total item count to this result.
    #[must_use]
    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }

    /// Returns `true` if there are more pages to fetch.
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }
}

impl<T> From<Vec<T>> for LoaderResult<T> {
    fn from(items: Vec<T>) -> Self {
        Self::done(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_has_no_cursor() {
        let result = LoaderResult::done(vec![1, 2, 3]);
        assert!(!result.has_more());
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.next_cursor, None);
        assert_eq!(result.total, None);
    }

    #[test]
    fn page_has_cursor() {
        let result = LoaderResult::page(vec![1, 2], "next");
        assert!(result.has_more());
        assert_eq!(result.next_cursor.as_deref(), Some("next"));
    }

    #[test]
    fn with_total_sets_total() {
        let result = LoaderResult::done(vec![1]).with_total(42);
        assert_eq!(result.total, Some(42));
    }

    #[test]
    fn from_vec_creates_done_result() {
        let result: LoaderResult<i32> = vec![1, 2, 3].into();
        assert!(!result.has_more());
        assert_eq!(result.items.len(), 3);
    }
}
