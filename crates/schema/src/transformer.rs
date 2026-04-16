use serde::{Deserialize, Serialize};

/// Value transformer applied before validation/runtime use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Transformer {
    /// Trim surrounding whitespace.
    Trim,
    /// Convert string to lowercase.
    Lowercase,
    /// Convert string to uppercase.
    Uppercase,
    /// Replace substring occurrences.
    Replace {
        /// Source string.
        from: String,
        /// Replacement string.
        to: String,
    },
}
