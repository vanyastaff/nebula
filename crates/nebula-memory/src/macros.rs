//! Public macros for nebula-memory crate

// Re-export arena macros
#[cfg(feature = "arena")]
pub use crate::arena_alloc;
#[cfg(feature = "arena")]
pub use crate::arena_alloc_or;
#[cfg(feature = "arena")]
pub use crate::arena_config;
#[cfg(feature = "arena")]
pub use crate::arena_debug;
#[cfg(feature = "arena")]
pub use crate::arena_str;
#[cfg(feature = "arena")]
pub use crate::arena_struct;
#[cfg(feature = "arena")]
pub use crate::arena_vec;
#[cfg(feature = "arena")]
pub use crate::bench_arena;
#[cfg(feature = "arena")]
pub use crate::impl_arena_alloc;
#[cfg(feature = "arena")]
pub use crate::local_alloc;
#[cfg(feature = "arena")]
pub use crate::shared_arena;
#[cfg(feature = "arena")]
pub use crate::strict_arena;
#[cfg(feature = "arena")]
pub use crate::try_arena_alloc;
#[cfg(feature = "arena")]
pub use crate::typed_arena;
#[cfg(feature = "arena")]
pub use crate::with_arena;

/// Create a memory pool with initial configuration
///
/// # Examples
/// ```
/// use nebula_memory::pool_config;
///
/// let pool = pool_config! {
///     initial_capacity: 100,
///     max_capacity: 1000,
///     create_fn: || Vec::<u8>::with_capacity(1024),
/// };
/// ```
#[macro_export]
macro_rules! pool_config {
    ($($field:ident: $value:expr),* $(,)?) => {{
        $crate::pool::PoolConfig {
            $($field: $value,)*
            ..Default::default()
        }
    }};
}

/// Get or create a value from a pool
///
/// # Examples
/// ```
/// use nebula_memory::pool::ObjectPool;
/// use nebula_memory::pool_get;
///
/// let pool = ObjectPool::new(10, 100, || Vec::<u8>::new());
/// let mut vec = pool_get!(pool);
/// vec.push(42);
/// ```
#[macro_export]
macro_rules! pool_get {
    ($pool:expr) => {{
        $pool.get().expect("Pool exhausted")
    }};

    ($pool:expr, $default:expr) => {{
        $pool.get().unwrap_or_else(|_| $default)
    }};
}

/// Create a compute cache with configuration
///
/// # Examples
/// ```
/// use nebula_memory::cache_config;
///
/// let cache = cache_config! {
///     max_entries: 1000,
///     ttl: std::time::Duration::from_secs(300),
///     compute_fn: |key: &str| key.len(),
/// };
/// ```
#[macro_export]
macro_rules! cache_config {
    ($($field:ident: $value:expr),* $(,)?) => {{
        $crate::cache::CacheConfig {
            $($field: $value,)*
            ..Default::default()
        }
    }};
}

/// Get or compute a value in cache
///
/// # Examples
/// ```
/// use nebula_memory::cache::ComputeCache;
/// use nebula_memory::cache_get;
///
/// let cache = ComputeCache::new(100);
/// let value = cache_get!(cache, "key", || expensive_computation());
/// ```
#[macro_export]
macro_rules! cache_get {
    ($cache:expr, $key:expr, $compute:expr) => {{
        $cache.get_or_compute($key, $compute)
    }};
}

/// Create a COW value with automatic optimization
///
/// # Examples
/// ```
/// use nebula_memory::cow_value;
///
/// let data = vec![1, 2, 3, 4, 5];
/// let cow = cow_value!(data);
/// ```
#[macro_export]
macro_rules! cow_value {
    ($value:expr) => {{
        $crate::cow::SmartCow::from_owned($value)
    }};

    (borrowed: $value:expr) => {{
        $crate::cow::SmartCow::from_borrowed($value)
    }};
}

/// Profile memory usage of a code block
///
/// # Examples
/// ```
/// use nebula_memory::profile_memory;
///
/// let (result, stats) = profile_memory!({
///     let mut vec = Vec::with_capacity(1000);
///     for i in 0..1000 {
///         vec.push(i);
///     }
///     vec.len()
/// });
///
/// println!("Result: {}, Memory used: {} bytes", result, stats.bytes_allocated);
/// ```
#[macro_export]
macro_rules! profile_memory {
    ($body:expr) => {{
        let tracker = $crate::stats::MemoryTracker::new();
        tracker.start_tracking();

        let result = $body;

        tracker.stop_tracking();
        let stats = tracker.get_stats();

        (result, stats)
    }};
}

/// Assert memory usage is within bounds
///
/// # Examples
/// ```
/// use nebula_memory::assert_memory;
///
/// assert_memory!(
///     max_bytes: 1024 * 1024,  // 1MB
///     max_allocations: 1000,
///     {
///         // Your code here
///         let _data = vec![0u8; 1024];
///     }
/// );
/// ```
#[macro_export]
macro_rules! assert_memory {
    (max_bytes: $max_bytes:expr,max_allocations: $max_allocs:expr, $body:expr) => {{
        let (result, stats) = $crate::profile_memory!($body);

        assert!(
            stats.bytes_allocated <= $max_bytes,
            "Memory usage {} exceeds limit {}",
            stats.bytes_allocated,
            $max_bytes
        );

        assert!(
            stats.allocation_count <= $max_allocs,
            "Allocation count {} exceeds limit {}",
            stats.allocation_count,
            $max_allocs
        );

        result
    }};
}

/// Create a memory budget enforcer
///
/// # Examples
/// ```
/// use nebula_memory::memory_budget;
///
/// let budget = memory_budget!(
///     total: 10 * 1024 * 1024,  // 10MB
///     per_allocation: 1024 * 1024,  // 1MB max per allocation
/// );
/// ```
#[macro_export]
macro_rules! memory_budget {
    (total: $total:expr,per_allocation: $per_alloc:expr $(,)?) => {{
        $crate::budget::MemoryBudget::new($total, $per_alloc)
    }};
}

/// Defer memory cleanup until scope exit
///
/// # Examples
/// ```
/// use nebula_memory::defer_cleanup;
///
/// {
///     let data = vec![0u8; 1024];
///     defer_cleanup!(|| {
///         println!("Cleaning up {} bytes", data.len());
///     });
///     // Use data...
/// } // Cleanup runs here
/// ```
#[macro_export]
macro_rules! defer_cleanup {
    ($cleanup:expr) => {
        let _guard = $crate::utils::DeferGuard::new($cleanup);
    };
}

/// Create a scoped memory context
///
/// # Examples
/// ```
/// use nebula_memory::memory_scope;
///
/// memory_scope!("MyOperation", {
///     // All allocations in this scope are tracked
///     let data = vec![0u8; 1024];
///     process_data(&data);
/// });
/// ```
#[macro_export]
macro_rules! memory_scope {
    ($name:expr, $body:expr) => {{
        let _scope = $crate::profiling::MemoryScope::new($name);
        $body
    }};
}

/// Conditionally compile memory tracking code
///
/// # Examples
/// ```
/// use nebula_memory::when_profiling;
///
/// when_profiling! {
///     println!("Memory profiling is enabled");
///     // Additional profiling code
/// }
/// ```
#[macro_export]
macro_rules! when_profiling {
    ($body:expr) => {
        #[cfg(feature = "profiling")]
        {
            $body
        }
    };
}

/// Create a type-erased allocator
///
/// # Examples
/// ```
/// use nebula_memory::dyn_allocator;
///
/// let allocator = dyn_allocator!(StackAllocator::new(4096));
/// ```
#[macro_export]
macro_rules! dyn_allocator {
    ($allocator:expr) => {{
        Box::new($allocator) as Box<dyn $crate::allocator::CustomAllocator>
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macro_compilation() {
        // Just ensure macros compile correctly
        // Actual tests are in respective module test files
    }
}
