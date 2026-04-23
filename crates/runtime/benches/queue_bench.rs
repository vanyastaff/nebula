//! Throughput benchmark for `MemoryQueue` with N concurrent consumers.
//!
//! Issue #279 — under the previous `Arc<Mutex<mpsc::Receiver>>` design,
//! workers serialized on a single mutex inside `dequeue`, so adding consumers
//! gave no real parallelism. After switching to `async_channel` (multi-consumer),
//! throughput should scale roughly with worker count up to channel-internal
//! contention limits.
//!
//! The bench enqueues a fixed number of items and measures wall-clock time
//! for N=1, 2, 4, 8 workers to drain the queue.

use std::{sync::Arc, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nebula_engine::{MemoryQueue, TaskQueue, queue::DequeueResult};
use tokio::runtime::Builder;

const ITEMS: usize = 1024;
const POLL_TIMEOUT: Duration = Duration::from_millis(20);

async fn drain_with_workers(workers: usize) {
    let queue = Arc::new(MemoryQueue::new(ITEMS));
    for i in 0..ITEMS {
        queue.enqueue(serde_json::json!({ "i": i })).await.unwrap();
    }

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        handles.push(tokio::spawn(worker(queue)));
    }

    for h in handles {
        h.await.unwrap();
    }
}

async fn worker(queue: Arc<MemoryQueue>) {
    while let Ok(DequeueResult::Item { task_id, .. }) = queue.dequeue(POLL_TIMEOUT).await {
        queue.ack(&task_id).await.unwrap();
    }
}

fn bench_concurrent_dequeue(c: &mut Criterion) {
    let mut group = c.benchmark_group("queue/concurrent_dequeue");
    group.sample_size(20);

    for &workers in &[1usize, 2, 4, 8] {
        group.throughput(Throughput::Elements(ITEMS as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(workers),
            &workers,
            |b, &workers| {
                let rt = Builder::new_multi_thread()
                    .worker_threads(workers.max(2))
                    .enable_all()
                    .build()
                    .unwrap();
                b.iter(|| rt.block_on(drain_with_workers(workers)));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_concurrent_dequeue);
criterion_main!(benches);
