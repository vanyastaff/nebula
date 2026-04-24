//! Baseline — synthetic minimal resolve path for delta reporting.
//!
//! Not any hypothesis. Just `HashMap<Arc<str>, Box<dyn Any + Send + Sync>>
//! ::get + downcast_ref`, ahash-backed, fixed seed. Strategy §3.4 "200–500ns
//! typical" is the expectation for this shape. All three hypothesis benches
//! report as deltas from these numbers; absolute budget is ≤1µs at p95.

use std::any::Any;
use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};

type FastMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

fn make_map() -> FastMap<Arc<str>, Box<dyn Any + Send + Sync>> {
    let hasher = ahash::RandomState::with_seeds(0, 0, 0, 0);
    let mut m: FastMap<Arc<str>, Box<dyn Any + Send + Sync>> = FastMap::with_hasher(hasher);
    for i in 0..64 {
        let k: Arc<str> = Arc::from(format!("slot_{i}").as_str());
        m.insert(k, Box::new(i as u64));
    }
    m
}

fn bench_baseline(c: &mut Criterion) {
    let map = make_map();
    let key: Arc<str> = Arc::from("slot_42");
    let key_s: &str = &key;

    c.bench_function("baseline/resolve_then_downcast", |b| {
        b.iter(|| {
            let entry = map.get(black_box(key_s));
            let v = entry.and_then(|b| b.downcast_ref::<u64>());
            black_box(v)
        });
    });
}

criterion_group!(benches, bench_baseline);
criterion_main!(benches);
