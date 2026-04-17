//! Expression value wrapper — Task 14 replaces with real OnceLock-based parse.

use std::sync::Arc;

/// Unresolved expression source. Task 14 adds lazy parse + OnceLock.
#[derive(Debug, Clone)]
pub struct Expression {
    source: Arc<str>,
}

impl Expression {
    /// Wrap an expression source string.
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
        }
    }

    /// Return the raw expression source.
    pub fn source(&self) -> &str {
        &self.source
    }
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}
