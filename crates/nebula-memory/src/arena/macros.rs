//! Convenience macros for arena allocation

/// Allocate multiple values in an arena at once
///
/// # Examples
/// ```
/// use nebula_memory::arena::Arena;
/// use nebula_memory::arena_alloc;
///
/// let arena = Arena::new(Default::default());
/// let (x, y, z) = arena_alloc!(arena, 42u32, "hello", vec![1, 2, 3]);
/// assert_eq!(*x, 42);
/// assert_eq!(*y, "hello");
/// assert_eq!(*z, vec![1, 2, 3]);
/// ```
#[macro_export]
macro_rules! arena_alloc {
    ($arena:expr, $($value:expr),+ $(,)?) => {{
        ($(
            $arena.alloc($value).expect("Arena allocation failed")
        ),+)
    }};
}

/// Allocate values in an arena with error handling
///
/// # Examples
/// ```
/// use nebula_memory::arena::Arena;
/// use nebula_memory::try_arena_alloc;
///
/// let arena = Arena::new(Default::default());
/// let result = try_arena_alloc!(arena, 42u32, "hello");
/// match result {
///     Ok((x, y)) => {
///         assert_eq!(*x, 42);
///         assert_eq!(*y, "hello");
///     },
///     Err(e) => panic!("Allocation failed: {:?}", e),
/// }
/// ```
#[macro_export]
macro_rules! try_arena_alloc {
    ($arena:expr, $($value:expr),+ $(,)?) => {{
        (|| -> Result<_, $crate::error::MemoryError> {
            Ok(($(
                $arena.alloc($value)?
            ),+))
        })()
    }};
}

/// Create an arena-allocated vector with initial values
///
/// # Examples
/// ```
/// use nebula_memory::{arena::Arena, arena_vec};
///
/// let arena = Arena::new(Default::default());
/// let vec = arena_vec![arena; 1, 2, 3, 4, 5];
/// assert_eq!(&*vec, &[1, 2, 3, 4, 5]);
/// ```
#[macro_export]
macro_rules! arena_vec {
    ($arena:expr; $($value:expr),* $(,)?) => {{
        let values = [$($value),*];
        $arena.alloc_slice(&values).expect("Arena allocation failed")
    }};

    ($arena:expr; $value:expr; $count:expr) => {{
        let values = vec![$value; $count];
        $arena.alloc_slice(&values).expect("Arena allocation failed")
    }};
}

/// Create an arena-allocated string
///
/// # Examples
/// ```
/// use nebula_memory::arena::Arena;
/// use nebula_memory::arena_str;
///
/// let arena = Arena::new(Default::default());
/// let s = arena_str!(arena, "Hello, {}", "world");
/// assert_eq!(s, "Hello, world");
/// ```
#[macro_export]
macro_rules! arena_str {
    ($arena:expr, $fmt:expr $(, $arg:expr)*) => {{
        let string = format!($fmt $(, $arg)*);
        $arena.alloc_str(&string).expect("Arena allocation failed")
    }};
}

/// Allocate in the thread-local arena
///
/// # Examples
/// ```
/// use nebula_memory::{local_alloc, reset_local_arena};
///
/// let x = local_alloc!(42u32);
/// let y = local_alloc!("hello");
/// assert_eq!(*x, 42);
/// assert_eq!(*y, "hello");
///
/// reset_local_arena();
/// ```
#[macro_export]
macro_rules! local_alloc {
    ($value:expr) => {{ $crate::arena::alloc_local($value).expect("Local arena allocation failed") }};
}

/// Create a typed arena with pre-allocated values (scoped API)
///
/// # Examples
/// ```
/// use nebula_memory::typed_arena;
///
/// typed_arena! {
///     u32 => [1, 2, 3, 4, 5] => |arena, values| {
///         assert_eq!(values.len(), 5);
///         for (i, &val) in values.iter().enumerate() {
///             assert_eq!(**val, i as u32 + 1);
///         }
///     }
/// };
/// ```
///
/// The closure receives the arena and pre-allocated values.
/// References are valid only within the closure scope.
#[macro_export]
macro_rules! typed_arena {
    ($type:ty => [$($value:expr),* $(,)?] => |$arena:ident, $values:ident| $body:block) => {{
        let $arena = $crate::arena::TypedArena::<$type>::new();
        let $values = vec![
            $($arena.alloc($value).expect("TypedArena allocation failed")),*
        ];
        $body
    }};
}

/// Execute code with a temporary arena that's automatically reset
///
/// # Examples
/// ```
/// use nebula_memory::with_arena;
///
/// let result = with_arena!(|arena| {
///     let x = arena.alloc(42).unwrap();
///     let y = arena.alloc(100).unwrap();
///     *x + *y
/// });
///
/// assert_eq!(result, 142);
/// ```
#[macro_export]
macro_rules! with_arena {
    (| $arena:ident | $body:expr) => {{
        let mut $arena = $crate::arena::Arena::new(Default::default());
        let result = $body;
        $arena.reset();
        result
    }};

    ($config:expr, | $arena:ident | $body:expr) => {{
        let mut $arena = $crate::arena::Arena::new($config);
        let result = $body;
        $arena.reset();
        result
    }};
}

/// Create an arena with a specific configuration
///
/// # Examples
/// ```
/// use nebula_memory::arena_config;
///
/// let arena = arena_config! {
///     initial_size: 8192,
///     growth_factor: 1.5,
///     track_stats: true,
///     zero_memory: false,
/// };
/// ```
#[macro_export]
macro_rules! arena_config {
    ($($field:ident: $value:expr),* $(,)?) => {{
        $crate::arena::ArenaConfig {
            $($field: $value,)*
            ..Default::default()
        }
    }};
}

/// Allocate and initialize a struct in an arena
///
/// # Examples
/// ```
/// use nebula_memory::arena::Arena;
/// use nebula_memory::arena_struct;
///
/// struct Person {
///     name: &'static str,
///     age: u32,
/// }
///
/// let arena = Arena::new(Default::default());
/// let person = arena_struct!(arena, Person { name: "Alice", age: 30 });
///
/// assert_eq!(person.name, "Alice");
/// assert_eq!(person.age, 30);
/// ```
#[macro_export]
macro_rules! arena_struct {
    ($arena:expr, $struct_name:ident { $($field:ident: $value:expr),* $(,)? }) => {{
        $arena.alloc($struct_name {
            $($field: $value),*
        }).expect("Arena allocation failed")
    }};
}

/// Benchmark arena allocation
///
/// # Examples
/// ```
/// use nebula_memory::arena::Arena;
/// use nebula_memory::bench_arena;
///
/// let arena = Arena::new(Default::default());
/// let (result, duration) = bench_arena!(arena, {
///     let mut sum = 0;
///     for i in 0..1000 {
///         let x = arena.alloc(i).unwrap();
///         sum += *x;
///     }
///     sum
/// });
///
/// println!("Sum: {}, Time: {:?}", result, duration);
/// ```
#[macro_export]
macro_rules! bench_arena {
    ($arena:expr, $body:expr) => {{
        let start = std::time::Instant::now();
        let result = $body;
        let duration = start.elapsed();
        (result, duration)
    }};
}

/// Create a thread-safe arena shared between threads
///
/// # Examples
/// ```
/// use nebula_memory::shared_arena;
/// use std::thread;
///
/// let arena = shared_arena!(initial_size: 8192);
///
/// let arena_clone = arena.clone();
/// let handle = thread::spawn(move || {
///     let x = arena_clone.alloc(42).unwrap();
///     *x
/// });
///
/// let result = handle.join().unwrap();
/// assert_eq!(result, 42);
/// ```
#[macro_export]
macro_rules! shared_arena {
    () => {{
        std::sync::Arc::new(
            $crate::arena::ThreadSafeArena::new(Default::default())
        )
    }};

    ($($field:ident: $value:expr),* $(,)?) => {{
        std::sync::Arc::new(
            $crate::arena::ThreadSafeArena::new($crate::arena::ArenaConfig {
                $($field: $value,)*
                ..Default::default()
            })
        )
    }};
}

/// Allocate in an arena or return a default value
///
/// # Examples
/// ```
/// use nebula_memory::arena::Arena;
/// use nebula_memory::arena_alloc_or;
///
/// let arena = Arena::new(Default::default());
/// let x = arena_alloc_or!(arena, Ok::<i32, &str>(42), -1);
/// assert_eq!(*x, 42);
/// ```
#[macro_export]
macro_rules! arena_alloc_or {
    ($arena:expr, $value:expr, $default:expr) => {{
        match $arena.alloc($value) {
            Ok(val) => val,
            Err(_) => $arena.alloc($default).expect("Default allocation failed"),
        }
    }};
}

/// Define arena allocation methods for a custom type
///
/// # Examples
/// ```
/// use nebula_memory::arena::Arena;
/// use nebula_memory::impl_arena_alloc;
///
/// struct MyAllocator {
///     arena: Arena,
/// }
///
/// impl_arena_alloc!(MyAllocator);
///
/// let allocator = MyAllocator { arena: Arena::new(Default::default()) };
///
/// let x = allocator.alloc(42).unwrap();
/// assert_eq!(*x, 42);
/// ```
#[macro_export]
macro_rules! impl_arena_alloc {
    ($type:ty) => {
        impl $type {
            pub fn alloc<T>(&self, value: T) -> Result<&mut T, $crate::error::MemoryError> {
                self.arena.alloc(value)
            }

            pub fn alloc_slice<T>(
                &self,
                slice: &[T],
            ) -> Result<&mut [T], $crate::error::MemoryError>
            where
                T: Copy,
            {
                self.arena.alloc_slice(slice)
            }

            pub fn alloc_str(&self, s: &str) -> Result<&str, $crate::error::MemoryError> {
                self.arena.alloc_str(s)
            }
        }
    };
}

/// Debug print arena statistics
///
/// # Examples
/// ```
/// use nebula_memory::arena::Arena;
/// use nebula_memory::arena_debug;
///
/// let arena = Arena::new(arena_config! {
///     track_stats: true,
/// });
///
/// let _ = arena.alloc(42).unwrap();
/// arena_debug!(arena);
/// ```
#[macro_export]
macro_rules! arena_debug {
    ($arena:expr) => {{
        if cfg!(debug_assertions) {
            let stats = $arena.stats().snapshot();
            eprintln!("Arena Debug Info:");
            eprintln!("  Bytes allocated: {}", stats.bytes_allocated);
            eprintln!("  Bytes used: {}", stats.bytes_used);
            eprintln!("  Allocations: {}", stats.allocations);
            eprintln!("  Utilization: {:.1}%", stats.utilization_ratio * 100.0);
        }
    }};
}

/// Create an arena that panics on allocation failure
///
/// # Examples
/// ```
/// use nebula_memory::strict_arena;
///
/// let arena = strict_arena!();
/// let x = arena.alloc(42); // Returns &mut i32, panics on failure
/// assert_eq!(*x, 42);
/// ```
#[macro_export]
macro_rules! strict_arena {
    () => {{ $crate::arena::StrictArena::new($crate::arena::Arena::new(Default::default())) }};

    ($config:expr) => {{ $crate::arena::StrictArena::new($crate::arena::Arena::new($config)) }};
}

// Helper wrapper for strict arena
pub struct StrictArena<A> {
    inner: A,
}

impl<A> StrictArena<A> {
    pub fn new(arena: A) -> Self {
        StrictArena { inner: arena }
    }
}

impl<A: crate::arena::ArenaAllocate> StrictArena<A> {
    pub fn alloc<T>(&self, value: T) -> &mut T {
        self.inner.alloc(value).expect("Arena allocation failed")
    }

    pub fn alloc_slice<T>(&self, slice: &[T]) -> &mut [T]
    where
        T: Copy,
    {
        self.inner
            .alloc_slice(slice)
            .expect("Arena allocation failed")
    }
}

#[cfg(test)]
mod tests {
    use crate::arena::*;

    #[test]
    fn test_arena_alloc_macro() {
        let arena = Arena::new(Default::default());
        let (x, y, z) = arena_alloc!(arena, 42u32, "hello", vec![1, 2, 3]);

        assert_eq!(*x, 42);
        assert_eq!(*y, "hello");
        assert_eq!(*z, vec![1, 2, 3]);
    }

    #[test]
    fn test_try_arena_alloc_macro() {
        let arena = Arena::new(Default::default());
        let result = try_arena_alloc!(arena, 100u64, "world");

        assert!(result.is_ok());
        let (x, y) = result.unwrap();
        assert_eq!(*x, 100);
        assert_eq!(*y, "world");
    }

    #[test]
    fn test_arena_vec_macro() {
        let arena = Arena::new(Default::default());

        let vec1 = arena_vec![arena; 1, 2, 3, 4, 5];
        assert_eq!(&*vec1, &[1, 2, 3, 4, 5]);

        let vec2 = arena_vec![arena; 42; 10];
        assert_eq!(vec2.len(), 10);
        assert!(vec2.iter().all(|&x| x == 42));
    }

    #[test]
    fn test_arena_str_macro() {
        let arena = Arena::new(Default::default());

        let s1 = arena_str!(arena, "Hello, {}", "world");
        assert_eq!(s1, "Hello, world");

        let s2 = arena_str!(arena, "The answer is {}", 42);
        assert_eq!(s2, "The answer is 42");
    }

    #[test]
    fn test_local_alloc_macro() {
        let x = local_alloc!(42u32);
        let y = local_alloc!("local arena");

        assert_eq!(*x, 42);
        assert_eq!(*y, "local arena");

        reset_local_arena();
    }

    #[test]
    fn test_typed_arena_macro() {
        typed_arena! {
            String => ["one".to_string(), "two".to_string(), "three".to_string()] => |arena, values| {
                assert_eq!(values.len(), 3);
                assert_eq!(&**values[0], "one");
                assert_eq!(&**values[1], "two");
                assert_eq!(&**values[2], "three");

                // Arena is still accessible within the scope
                let extra = arena.alloc("extra".to_string()).unwrap();
                assert_eq!(&**extra, "extra");
            }
        }
    }

    #[test]
    fn test_with_arena_macro() {
        let sum = with_arena!(|arena| {
            let mut total = 0;
            for i in 0..10 {
                let x = arena.alloc(i).unwrap();
                total += *x;
            }
            total
        });

        assert_eq!(sum, 45);
    }

    #[test]
    fn test_arena_config_macro() {
        let config = arena_config! {
            initial_size: 8192,
            growth_factor: 1.5,
            track_stats: true,
        };

        assert_eq!(config.initial_size, 8192);
        assert_eq!(config.growth_factor, 1.5);
        assert!(config.track_stats);
    }

    #[test]
    fn test_arena_struct_macro() {
        #[derive(Debug, PartialEq)]
        struct TestStruct {
            x: i32,
            y: String,
        }

        let arena = Arena::new(Default::default());
        let s = arena_struct!(
            arena,
            TestStruct {
                x: 42,
                y: "test".to_string()
            }
        );

        assert_eq!(s.x, 42);
        assert_eq!(s.y, "test");
    }

    #[test]
    fn test_bench_arena_macro() {
        let arena = Arena::new(Default::default());
        let (result, duration) = bench_arena!(arena, {
            let x = arena.alloc(100).unwrap();
            let y = arena.alloc(200).unwrap();
            *x + *y
        });

        assert_eq!(result, 300);
        assert!(duration.as_nanos() > 0);
    }

    #[test]
    fn test_shared_arena_macro() {
        let arena1 = shared_arena!();
        let arena2 = shared_arena!(initial_size: 8192, track_stats: true);

        let x = arena1.alloc(42).unwrap();
        assert_eq!(*x, 42);

        let y = arena2.alloc(100).unwrap();
        assert_eq!(*y, 100);
    }

    #[test]
    fn test_arena_alloc_or_macro() {
        let arena = Arena::new(Default::default());

        let x = arena_alloc_or!(arena, 42, 0);
        assert_eq!(*x, 42);
    }

    #[test]
    fn test_impl_arena_alloc_macro() {
        struct CustomAllocator {
            arena: Arena,
        }

        impl_arena_alloc!(CustomAllocator);

        let allocator = CustomAllocator {
            arena: Arena::new(Default::default()),
        };

        let x = allocator.alloc(42).unwrap();
        assert_eq!(*x, 42);

        let slice = allocator.alloc_slice(&[1, 2, 3]).unwrap();
        assert_eq!(slice, &[1, 2, 3]);

        let s = allocator.alloc_str("hello").unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_strict_arena_macro() {
        let arena = strict_arena!();
        let x = arena.alloc(42);
        assert_eq!(*x, 42);

        let slice = arena.alloc_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(slice, &[1, 2, 3, 4, 5]);
    }
}
