//! Trait for objects that can be pooled

/// Trait for objects that can be pooled
///
/// # Example
/// ```
/// use nebula_memory::pool::Poolable;
///
/// struct Connection {
///     id: u64,
///     buffer: Vec<u8>,
/// }
///
/// impl Poolable for Connection {
///     fn reset(&mut self) {
///         self.buffer.clear();
///     }
///
///     fn is_reusable(&self) -> bool {
///         self.buffer.capacity() < 1_000_000 // Don't reuse huge buffers
///     }
/// }
/// ```
pub trait Poolable: Send + 'static {
    /// Reset object to initial state
    ///
    /// This method is called when an object is returned to the pool.
    /// It should clear any state that shouldn't persist between uses.
    fn reset(&mut self);

    /// Check if object is reusable
    ///
    /// Return false if the object should be discarded instead of pooled.
    /// This is useful for objects that may become unusable or too large.
    fn is_reusable(&self) -> bool {
        true
    }

    /// Get priority for retention (0-255, higher = keep longer)
    ///
    /// Used by `PriorityPool` to determine which objects to keep
    /// when the pool needs to shrink.
    fn priority(&self) -> u8 {
        128
    }

    /// Validate object state
    ///
    /// Called when `validate_on_return` is enabled in pool config.
    /// Return false to discard the object.
    fn validate(&self) -> bool {
        true
    }

    /// Get approximate memory usage in bytes
    ///
    /// Used for memory budgeting and statistics.
    fn memory_usage(&self) -> usize {
        core::mem::size_of_val(self)
    }

    /// Compress object to reduce memory usage when system is under pressure
    ///
    /// Called when memory pressure is high and object needs to be kept in pool.
    /// Should attempt to reduce memory footprint by releasing unused capacity.
    /// Return true if object was compressed, false if compression not
    /// supported.
    #[cfg(feature = "adaptive")]
    fn compress(&mut self) -> bool {
        false
    }
}

// Standard library implementations

impl Poolable for String {
    fn reset(&mut self) {
        self.clear();
    }

    fn is_reusable(&self) -> bool {
        // Don't pool strings with huge capacity
        self.capacity() < 1_000_000
    }

    fn memory_usage(&self) -> usize {
        core::mem::size_of::<String>() + self.capacity()
    }

    #[cfg(feature = "adaptive")]
    fn compress(&mut self) -> bool {
        if self.capacity() > self.len() * 2 && self.capacity() > 64 {
            let _old_capacity = self.capacity();
            *self = self.clone();
            true
        } else {
            false
        }
    }
}

// Общая реализация для всех типов Vec<T>
impl<T> Poolable for Vec<T>
where
    T: Send + 'static,
    T: core::any::Any,
    // Используем специализацию на уровне типа
    Self: core::marker::Sized,
    T: std::fmt::Debug,
    T: 'static,
{
    fn reset(&mut self) {
        self.clear();
    }

    fn is_reusable(&self) -> bool {
        // Специальный случай для Vec<u8>
        if core::any::TypeId::of::<T>() == core::any::TypeId::of::<u8>() {
            return self.capacity() < 10_000_000; // 10MB limit для Vec<u8>
        }

        // Общий случай для других типов
        self.capacity() < 100_000
    }

    fn memory_usage(&self) -> usize {
        core::mem::size_of::<Vec<T>>() + (self.capacity() * core::mem::size_of::<T>())
    }

    #[cfg(feature = "adaptive")]
    fn compress(&mut self) -> bool {
        // Специальный случай для Vec<u8>
        let threshold = if core::any::TypeId::of::<T>() == core::any::TypeId::of::<u8>() {
            1024 // Больший порог для Vec<u8>
        } else {
            128
        };

        if self.capacity() > self.len() * 2 && self.capacity() > threshold {
            let old_capacity = self.capacity();
            self.shrink_to_fit();
            old_capacity != self.capacity()
        } else {
            false
        }
    }
}

impl<K, V> Poolable for std::collections::HashMap<K, V>
where
    K: Send + 'static,
    V: Send + 'static,
{
    fn reset(&mut self) {
        self.clear();
    }

    fn is_reusable(&self) -> bool {
        self.capacity() < 10_000
    }

    fn memory_usage(&self) -> usize {
        // Approximate - HashMap overhead is complex
        core::mem::size_of::<Self>()
            + (self.capacity() * (core::mem::size_of::<K>() + core::mem::size_of::<V>() + 16))
    }

    #[cfg(feature = "adaptive")]
    fn compress(&mut self) -> bool {
        // Note: shrink_to_fit requires additional trait bounds on HashMap
        // Returning false for now (no compression performed)
        false
    }
}

/// Extension trait for poolable builders
pub trait PoolableBuilder: Poolable {
    /// Create a new instance
    fn build() -> Self;
}

/// Macro to implement Poolable for simple types
#[macro_export]
macro_rules! impl_poolable {
    ($type:ty, $reset:expr) => {
        impl $crate::pool::Poolable for $type {
            fn reset(&mut self) {
                $reset(self);
            }
        }
    };

    ($type:ty, $reset:expr, $reusable:expr) => {
        impl $crate::pool::Poolable for $type {
            fn reset(&mut self) {
                $reset(self);
            }

            fn is_reusable(&self) -> bool {
                $reusable(self)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_poolable() {
        let mut s = String::from("hello world");
        assert!(s.is_reusable());

        s.reset();
        assert_eq!(s, "");
        assert!(s.capacity() > 0); // Capacity is preserved
    }

    #[test]
    fn test_vec_poolable() {
        let mut v = vec![1, 2, 3, 4, 5];
        let cap = v.capacity();

        v.reset();
        assert!(v.is_empty());
        assert_eq!(v.capacity(), cap); // Capacity preserved
    }

    #[test]
    fn test_large_vec_not_reusable() {
        let mut v = Vec::with_capacity(10_000_001);
        v.push(1u8);
        assert!(!v.is_reusable());
    }

    #[cfg(feature = "adaptive")]
    #[test]
    fn test_string_compression() {
        let mut s = String::with_capacity(1000);
        s.push_str("test");
        assert_eq!(s.capacity(), 1000);

        let result = s.compress();
        assert!(result);
        assert!(s.capacity() < 1000);
        assert_eq!(s, "test");
    }

    #[cfg(feature = "adaptive")]
    #[test]
    fn test_vec_compression() {
        let mut v = Vec::<u8>::with_capacity(10000);
        v.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(v.capacity(), 10000);

        let result = v.compress();
        assert!(result);
        assert!(v.capacity() < 10000);
        assert_eq!(v, vec![1, 2, 3, 4, 5]);
    }

    #[cfg(feature = "adaptive")]
    #[test]
    fn test_hashmap_compression() {
        use std::collections::HashMap;

        let mut map = HashMap::<i32, i32>::with_capacity(1000);
        map.insert(1, 1);
        map.insert(2, 2);
        assert!(map.capacity() >= 1000);

        let result = map.compress();
        assert!(result);
        assert!(map.capacity() < 1000);
        assert_eq!(map.len(), 2);
    }
}
