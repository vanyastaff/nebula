//! Boolean scalar type for nebula-value
//!
//! Provides a simple wrapper around bool for consistency with other scalar types.

use std::fmt;

/// Boolean value
///
/// Simple wrapper around bool for consistency with other nebula-value scalar types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Boolean {
    inner: bool,
}

impl Boolean {
    /// Create a new Boolean from a bool
    pub const fn new(value: bool) -> Self {
        Self { inner: value }
    }

    /// Get the inner bool value
    pub const fn value(&self) -> bool {
        self.inner
    }

    /// Create a true Boolean
    pub const fn r#true() -> Self {
        Self { inner: true }
    }

    /// Create a false Boolean
    pub const fn r#false() -> Self {
        Self { inner: false }
    }

    /// Check if this is true
    pub const fn is_true(&self) -> bool {
        self.inner
    }

    /// Check if this is false
    pub const fn is_false(&self) -> bool {
        !self.inner
    }
}

impl From<bool> for Boolean {
    fn from(value: bool) -> Self {
        Self::new(value)
    }
}

impl From<Boolean> for bool {
    fn from(value: Boolean) -> Self {
        value.inner
    }
}

impl fmt::Display for Boolean {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl Default for Boolean {
    fn default() -> Self {
        Self::new(false)
    }
}

// Implement logical operations
impl std::ops::Not for Boolean {
    type Output = Boolean;

    fn not(self) -> Self::Output {
        Boolean::new(!self.inner)
    }
}

impl std::ops::BitAnd for Boolean {
    type Output = Boolean;

    fn bitand(self, rhs: Self) -> Self::Output {
        Boolean::new(self.inner & rhs.inner)
    }
}

impl std::ops::BitOr for Boolean {
    type Output = Boolean;

    fn bitor(self, rhs: Self) -> Self::Output {
        Boolean::new(self.inner | rhs.inner)
    }
}

impl std::ops::BitXor for Boolean {
    type Output = Boolean;

    fn bitxor(self, rhs: Self) -> Self::Output {
        Boolean::new(self.inner ^ rhs.inner)
    }
}
