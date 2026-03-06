//! End-to-end subscriber throughput benchmarks for nebula-eventbus.

#![expect(
    clippy::excessive_nesting,
    reason = "Criterion async benchmark closures require nested setup and loops"
)]

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_eventbus::{EventBus, EventFilter, SubscriptionScope};

#[derive(Clone)]
struct BenchEvent {
    execution_id: &'static str,
    value: u64,
}

impl nebula_eventbus::ScopedEvent for BenchEvent {
    fn execution_id(&self) -> Option<&str> {
        Some(self.execution_id)
    }
}

fn bench_subscriber_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime for async criterion benches");
    let mut group = c.benchmark_group("eventbus/subscriber_throughput");

    for &batch in &[1_000_u64, 10_000_u64] {
        group.bench_with_input(BenchmarkId::new("plain", batch), &batch, |b, &batch| {
            b.to_async(&rt).iter(|| async move {
                let bus = EventBus::new((batch as usize) + 16);
                let mut sub = bus.subscribe();

                for i in 0..batch {
                    let _ = bus.emit(BenchEvent {
                        execution_id: "exec-1",
                        value: i,
                    });
                }

                for _ in 0..batch {
                    let ev = sub
                        .recv()
                        .await
                        .expect("subscriber should receive all events");
                    black_box(ev.value);
                }
            });
        });

        group.bench_with_input(BenchmarkId::new("scoped", batch), &batch, |b, &batch| {
            b.to_async(&rt).iter(|| async move {
                let bus = EventBus::new((batch as usize) + 16);
                let mut sub = bus.subscribe_scoped(SubscriptionScope::execution("exec-1"));

                for i in 0..batch {
                    let exec = if i.is_multiple_of(2) {
                        "exec-1"
                    } else {
                        "exec-2"
                    };
                    let _ = bus.emit(BenchEvent {
                        execution_id: exec,
                        value: i,
                    });
                }

                let expected = batch / 2 + (batch % 2);
                for _ in 0..expected {
                    let ev = sub
                        .recv()
                        .await
                        .expect("scoped subscriber should receive matching events");
                    black_box(ev.value);
                }
            });
        });

        group.bench_with_input(BenchmarkId::new("filtered", batch), &batch, |b, &batch| {
            b.to_async(&rt).iter(|| async move {
                let bus = EventBus::new((batch as usize) + 16);
                let mut sub = bus.subscribe_filtered(EventFilter::custom(|event: &BenchEvent| {
                    event.value.is_multiple_of(3)
                }));

                for i in 0..batch {
                    let _ = bus.emit(BenchEvent {
                        execution_id: "exec-1",
                        value: i,
                    });
                }

                let expected = batch.div_ceil(3);
                for _ in 0..expected {
                    let ev = sub
                        .recv()
                        .await
                        .expect("filtered subscriber should receive matching events");
                    black_box(ev.value);
                }
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_subscriber_throughput);
criterion_main!(benches);
