//! Canonical schema values with explicit authoring and resolution capabilities.

mod budget;
mod canonical;
mod tree;
mod tree_canonical;
mod wire;

pub use canonical::canonical_json_v1;
pub use nebula_validator::foundation::FieldPath as ValuePath;
pub use tree::{AuthoredValue, CompiledValue, ResolvedValue, ScalarValue, ValueTree};

/// Reserved key in the explicit authoring shorthand.
pub const EXPRESSION_KEY: &str = "$expr";

/// Maximum logical depth accepted at value custody boundaries.
pub const MAX_VALUE_DEPTH: u8 = 64;

/// Maximum number of data nodes accepted at a value custody boundary.
pub const MAX_VALUE_NODES: usize = 65_536;

/// Maximum cumulative UTF-8 bytes in data keys and string values.
pub const MAX_VALUE_TEXT_BYTES: usize = 1_048_576;

/// Maximum number of retained expressions accepted at a value custody boundary.
pub const MAX_EXPRESSION_ENTRIES: usize = 4_096;

/// Maximum cumulative UTF-8 bytes in expression paths and sources.
pub const MAX_EXPRESSION_TEXT_BYTES: usize = 1_048_576;

/// Canonical tree encoding version. Persisted graph JSON uses its separate v1 codec.
pub const VALUE_CANON_VERSION: u16 = 2;

/// A content address for a secret-free canonical tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentId([u8; 32]);

impl ContentId {
    pub(crate) fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    /// Borrow the raw digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Display for ContentId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
