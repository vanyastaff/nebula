//! Credential filtering for list operations

use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Filter for listing credentials
///
/// Used with [`StorageProvider::list()`] to filter credentials by tags
/// and date ranges.
///
/// [`StorageProvider::list()`]: crate::traits::StorageProvider::list
///
/// # Examples
///
/// ```
/// use nebula_credential::core::CredentialFilter;
/// use chrono::Utc;
/// use std::collections::HashMap;
///
/// // Filter by tags
/// let mut tags = HashMap::new();
/// tags.insert("environment".to_string(), "production".to_string());
/// let filter = CredentialFilter {
///     tags: Some(tags),
///     created_after: None,
///     created_before: None,
/// };
///
/// // Filter by date range
/// let now = Utc::now();
/// let filter = CredentialFilter {
///     tags: None,
///     created_after: Some(now - chrono::Duration::days(30)),
///     created_before: Some(now),
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct CredentialFilter {
    /// Filter by tags (all tags must match)
    pub tags: Option<HashMap<String, String>>,

    /// Filter by creation date (inclusive)
    pub created_after: Option<DateTime<Utc>>,

    /// Filter by creation date (inclusive)
    pub created_before: Option<DateTime<Utc>>,
}

impl CredentialFilter {
    /// Create empty filter (matches all credentials)
    pub fn new() -> Self {
        Self::default()
    }

    /// Add tag filter
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Add created_after filter
    pub fn with_created_after(mut self, date: DateTime<Utc>) -> Self {
        self.created_after = Some(date);
        self
    }

    /// Add created_before filter
    pub fn with_created_before(mut self, date: DateTime<Utc>) -> Self {
        self.created_before = Some(date);
        self
    }
}
