//! Object pool with RAII lifecycle and automatic return
//!
//! Demonstrates `ObjectPool` — objects are automatically returned to the pool
//! when the `PooledValue` guard is dropped.

use nebula_memory::pool::{ObjectPool, PoolConfig, Poolable};

/// A reusable buffer that clears itself when returned to the pool.
#[derive(Debug)]
struct Buffer {
    data: Vec<u8>,
    id: usize,
}

impl Poolable for Buffer {
    fn reset(&mut self) {
        self.data.clear();
    }
}

fn main() {
    // === Example 1: Basic pool usage ===
    println!("=== 1. Basic pool usage ===");

    let next_id = std::sync::atomic::AtomicUsize::new(0);
    let pool = ObjectPool::new(5, move || {
        let id = next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        Buffer {
            data: Vec::with_capacity(1024),
            id,
        }
    });

    {
        let mut buf = pool.get().unwrap();
        buf.data.extend_from_slice(b"hello world");
        println!(
            "Checked out buffer #{}, data len: {}",
            buf.id,
            buf.data.len()
        );
        // buf is returned to pool when dropped here
    }

    // Get the same buffer back (recycled, data cleared by reset())
    let buf = pool.get().unwrap();
    println!(
        "Recycled buffer #{}, data len: {} (reset!)",
        buf.id,
        buf.data.len()
    );
    assert!(buf.data.is_empty(), "reset() should have cleared the data");

    // === Example 2: Bounded pool with exhaustion ===
    println!("\n=== 2. Bounded pool (capacity=2) ===");

    let config = PoolConfig::bounded(2);
    let pool = ObjectPool::with_config(config, || String::new());

    let s1 = pool.get().unwrap();
    let s2 = pool.get().unwrap();
    println!("Checked out 2 strings, available: {}", pool.available());

    // Third checkout fails — pool exhausted
    match pool.get() {
        Err(e) => println!("Pool exhausted (expected): {e}"),
        Ok(_) => unreachable!(),
    }

    // Return one → now available again
    drop(s1);
    println!("After returning one: available={}", pool.available());

    let _s3 = pool.get().unwrap();
    println!("Successfully checked out again");
    drop(s2);

    // === Example 3: Detaching from the pool ===
    println!("\n=== 3. Detach (take ownership) ===");

    let pool = ObjectPool::new(3, || vec![0u8; 256]);

    let guard = pool.get().unwrap();
    println!("Before detach: pool available={}", pool.available());

    let owned: Vec<u8> = guard.detach();
    println!(
        "After detach:  pool available={} (object not returned)",
        pool.available()
    );
    println!("Owned vec len: {}", owned.len());

    // === Example 4: Pool capacity info ===
    println!("\n=== 4. Pool capacity ===");

    let pool = ObjectPool::new(10, || String::with_capacity(64));
    println!(
        "Capacity: {}, Available: {}",
        pool.capacity(),
        pool.available()
    );

    let _items: Vec<_> = (0..5).map(|_| pool.get().unwrap()).collect();
    println!("After 5 checkouts: available={}", pool.available());

    drop(_items);
    println!("After returning all: available={}", pool.available());
}
