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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ephemeral_creation() {
        let ephemeral = Ephemeral::new(42);
        assert_eq!(*ephemeral.get(), 42);
    }

    #[test]
    fn test_ephemeral_get() {
        let ephemeral = Ephemeral::new("test-value");
        assert_eq!(*ephemeral.get(), "test-value");
    }

    #[test]
    fn test_ephemeral_get_mut() {
        let mut ephemeral = Ephemeral::new(10);
        *ephemeral.get_mut() = 20;
        assert_eq!(*ephemeral.get(), 20);
    }

    #[test]
    fn test_ephemeral_into_inner() {
        let ephemeral = Ephemeral::new(vec![1, 2, 3]);
        let inner = ephemeral.into_inner();
        assert_eq!(inner, vec![1, 2, 3]);
    }

    #[test]
    fn test_ephemeral_clone() {
        let original = Ephemeral::new(100);
        let cloned = original.clone();
        assert_eq!(*original.get(), *cloned.get());
    }

    #[test]
    fn test_ephemeral_default() {
        let ephemeral: Ephemeral<i32> = Ephemeral::default();
        assert_eq!(*ephemeral.get(), 0);
    }

    #[test]
    fn test_ephemeral_debug() {
        let ephemeral = Ephemeral::new("debug-test");
        let debug_str = format!("{:?}", ephemeral);
        assert!(debug_str.contains("Ephemeral"));
        assert!(debug_str.contains("debug-test"));
    }

    #[test]
    fn test_ephemeral_zero_copy_wrapper() {
        // Verify it's truly zero-copy (just a wrapper)
        let value = String::from("test");
        let ephemeral = Ephemeral::new(value);
        let extracted = ephemeral.into_inner();
        assert_eq!(extracted, "test");
    }
}
