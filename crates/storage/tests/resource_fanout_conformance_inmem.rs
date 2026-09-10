//! Shared-resource fanout conformance for the in-memory reference adapter.

#[macro_use]
#[path = "support/resource_fanout_oracle.rs"]
mod oracle;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use nebula_core::accessor::Clock;
use nebula_storage::inmem::InMemoryResourceRuntime;
use nebula_storage_port::Scope;
use nebula_storage_port::dto::{ResourceDeliveryId, SharedResourceId};

#[derive(Debug)]
struct ManualClock {
    wall: Mutex<DateTime<Utc>>,
    monotonic_origin: Instant,
    elapsed: Mutex<Duration>,
}

impl ManualClock {
    fn new() -> Self {
        Self {
            wall: Mutex::new(DateTime::from_timestamp(1_800_000_000, 0).expect("valid epoch")),
            monotonic_origin: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
        }
    }

    fn advance(&self, duration: Duration) {
        *self.wall.lock().expect("manual wall clock lock") +=
            chrono::TimeDelta::from_std(duration).expect("test duration fits chrono");
        *self.elapsed.lock().expect("manual elapsed lock") += duration;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.wall.lock().expect("manual wall clock lock")
    }

    fn monotonic(&self) -> Instant {
        self.monotonic_origin + *self.elapsed.lock().expect("manual elapsed lock")
    }
}

#[async_trait::async_trait]
impl oracle::ResourceFanoutExpiryControl for Arc<ManualClock> {
    async fn expire_source_for_test(&self, _scope: &Scope, _resource_id: SharedResourceId) {
        self.advance(Duration::from_secs(10));
    }

    async fn expire_delivery_for_test(&self, _scope: &Scope, _delivery_id: ResourceDeliveryId) {
        self.advance(Duration::from_secs(10));
    }
}

async fn runtime() -> (InMemoryResourceRuntime, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new());
    (InMemoryResourceRuntime::with_clock(clock.clone()), clock)
}

resource_fanout_conformance_suite!(runtime());
