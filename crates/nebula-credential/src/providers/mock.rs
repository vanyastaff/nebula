//! Mock storage provider for testing
//!
//! Provides in-memory storage with error simulation capabilities.

// Placeholder - will be implemented in Phase 2 (T011-T013)

/// Placeholder mock storage provider
///
/// This is a stub that will be fully implemented in Phase 2 (T011-T013)
#[derive(Clone, Debug)]
pub struct MockStorageProvider;

impl MockStorageProvider {
    /// Create new mock provider
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockStorageProvider {
    fn default() -> Self {
        Self::new()
    }
}
