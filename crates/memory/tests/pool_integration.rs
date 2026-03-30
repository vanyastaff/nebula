//! Integration tests for object pool lifecycle
//!
//! Validates RAII return, reset, exhaustion/recovery, and thread-safe access.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use nebula_memory::pool::{ObjectPool, PoolConfig, Poolable, ThreadSafePool};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct TrackedBuffer {
    data: Vec<u8>,
    reset_count: usize,
}

impl Poolable for TrackedBuffer {
    fn reset(&mut self) {
        self.data.clear();
        self.reset_count += 1;
    }
}

// ---------------------------------------------------------------------------
// RAII lifecycle
// ---------------------------------------------------------------------------

#[test]
fn pooled_value_returns_on_drop() {
    let pool = ObjectPool::new(3, || TrackedBuffer {
        data: Vec::new(),
        reset_count: 0,
    });

    assert_eq!(pool.available(), 3);

    {
        let mut val = pool.get().unwrap();
        val.data.extend_from_slice(b"hello");
        assert_eq!(pool.available(), 2);
    } // val dropped → returned to pool

    assert_eq!(pool.available(), 3);

    // Re-checkout should get a reset buffer
    let val = pool.get().unwrap();
    assert!(val.data.is_empty(), "data should be cleared by reset()");
    assert!(
        val.reset_count >= 1,
        "reset should have been called at least once"
    );
}

#[test]
fn detach_does_not_return_to_pool() {
    let pool = ObjectPool::new(5, || String::with_capacity(64));

    let guard = pool.get().unwrap();
    let _owned = guard.detach();

    // Pool lost one object permanently
    assert_eq!(pool.available(), 4);
}

// ---------------------------------------------------------------------------
// Bounded pool exhaustion and recovery
// ---------------------------------------------------------------------------

#[test]
fn bounded_pool_exhaustion_and_recovery() {
    let config = PoolConfig::bounded(3);
    let pool = ObjectPool::with_config(config, || Vec::<u8>::new());

    let a = pool.get().unwrap();
    let b = pool.get().unwrap();
    let c = pool.get().unwrap();

    // Pool exhausted
    assert!(pool.get().is_err());

    // Return one → can checkout again
    drop(a);
    let _d = pool.get().unwrap();

    // Still exhausted
    assert!(pool.get().is_err());

    drop(b);
    drop(c);
}

#[test]
fn unbounded_pool_grows_on_demand() {
    let config = PoolConfig::unbounded(2);
    let pool = ObjectPool::with_config(config, String::new);

    // Checkout more than initial capacity
    let items: Vec<_> = (0..10).map(|_| pool.get().unwrap()).collect();
    assert_eq!(items.len(), 10);
}

// ---------------------------------------------------------------------------
// ThreadSafePool
// ---------------------------------------------------------------------------

#[test]
fn thread_safe_pool_concurrent_checkout() {
    let pool = Arc::new(ThreadSafePool::new(50, || Vec::<u8>::with_capacity(128)));
    let checkout_count = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    for _ in 0..10 {
        let p = Arc::clone(&pool);
        let count = Arc::clone(&checkout_count);
        handles.push(thread::spawn(move || {
            for _ in 0..20 {
                let mut val = p.get().unwrap();
                val.push(42);
                count.fetch_add(1, Ordering::Relaxed);
                // val returned on drop
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(checkout_count.load(Ordering::Relaxed), 200);
    // All items returned
    assert_eq!(pool.available(), 50);
}

#[test]
fn thread_safe_pool_bounded_contention() {
    let config = PoolConfig::bounded(5);
    let pool = Arc::new(ThreadSafePool::with_config(config, || String::new()));
    let success = Arc::new(AtomicUsize::new(0));
    let failure = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    for _ in 0..20 {
        let p = Arc::clone(&pool);
        let s = Arc::clone(&success);
        let f = Arc::clone(&failure);
        handles.push(thread::spawn(move || match p.get() {
            Ok(val) => {
                s.fetch_add(1, Ordering::Relaxed);
                thread::sleep(std::time::Duration::from_millis(10));
                drop(val);
            }
            Err(_) => {
                f.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let total = success.load(Ordering::Relaxed) + failure.load(Ordering::Relaxed);
    assert_eq!(total, 20);
    // At most 5 concurrent checkouts, so some must have failed
    assert!(
        failure.load(Ordering::Relaxed) > 0,
        "bounded pool should reject some"
    );
}

// ---------------------------------------------------------------------------
// Pool with validation
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ValidatedItem {
    valid: bool,
}

impl Poolable for ValidatedItem {
    fn reset(&mut self) {
        self.valid = true;
    }

    fn validate(&self) -> bool {
        self.valid
    }
}

#[test]
fn pool_with_validation_on_return() {
    let config = PoolConfig {
        initial_capacity: 3,
        validate_on_return: true,
        pre_warm: false,
        ..Default::default()
    };
    let pool = ObjectPool::with_config(config, || ValidatedItem { valid: true });

    let mut item = pool.get().unwrap();
    item.valid = false;
    drop(item); // returned to pool, reset() will set valid=true

    // Should still be able to get a valid item
    let item = pool.get().unwrap();
    assert!(item.valid);
}
