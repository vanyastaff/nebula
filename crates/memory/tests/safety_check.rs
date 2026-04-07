use nebula_memory::pool::{ObjectPool, PoolConfig, Poolable};

#[derive(Debug)]
#[allow(dead_code)]
struct TestItem {
    id: usize,
}

impl Poolable for TestItem {
    fn reset(&mut self) {}
}

#[test]
fn test_multiple_borrows_safety() {
    let config = PoolConfig {
        initial_capacity: 10,
        pre_warm: false,
        ..Default::default()
    };
    let pool = ObjectPool::with_config(config, || TestItem { id: 0 });

    // multiple checkouts allowed locally because pool uses RefCell internally
    let item1 = pool.get().unwrap();
    let item2 = pool.get().unwrap();

    // Both items should point to the SAME pool
    assert_eq!(item1.pool() as *const _, item2.pool() as *const _);
    assert_eq!(item1.pool() as *const _, &pool as *const _);
    assert_eq!(item2.pool() as *const _, &pool as *const _);
}

#[test]
fn test_pool_exhaustion_multi() {
    let config = PoolConfig::bounded(2);
    let pool = ObjectPool::with_config(config, || TestItem { id: 0 });

    let _i1 = pool.get().unwrap();
    let _i2 = pool.get().unwrap();

    // Pool exhausted
    assert!(pool.get().is_err());

    drop(_i1);

    // Now available
    assert!(pool.get().is_ok());
}

// Ensure lifetime prevents use-after-free
// COMPILATION FAIL TEST - handled by compiletest usually, but here just documentation/verification mentally
// fn fail_use_after_free() {
//     let mut pool = ObjectPool::new(10, || TestItem { id: 0 });
//     let item = pool.get().unwrap();
//     drop(pool);
//     // println!("{:?}", item.id); // Should fail to compile: `pool` dropped while borrowed by `item`
// }
