
/// Wrapper for ephemeral data that should never be serialized
#[derive(Debug, Clone, Default)]
pub struct Ephemeral<T>(pub T);

impl<T> Ephemeral<T> {
    /// Create new ephemeral value
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Get inner value
    pub fn get(&self) -> &T {
        &self.0
    }

    /// Get mutable reference
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }

    /// Extract inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

// Ephemeral fields should use serde attributes on the struct field:
// #[serde(skip_serializing, skip_deserializing, default)]
