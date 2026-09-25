//! Resource status conformance for the in-memory reference adapter.

#[macro_use]
#[path = "support/resource_status_oracle.rs"]
mod oracle;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use nebula_core::accessor::Clock;
use nebula_storage::inmem::InMemoryResourceStatusStore;
use nebula_storage_port::dto::StatusWorkerId;

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
impl oracle::ResourceStatusTimeControl for Arc<ManualClock> {
    async fn pass(&self, duration: Duration) {
        self.advance(duration);
    }

    async fn expire_long_ago(&self, _worker: &StatusWorkerId) {
        // Every heartbeat the scenario holds expires with it; the scenario
        // renews the ones it still needs afterwards.
        self.advance(Duration::from_hours(3));
    }
}

async fn store() -> Option<(InMemoryResourceStatusStore, Arc<ManualClock>)> {
    let clock = Arc::new(ManualClock::new());
    Some((
        InMemoryResourceStatusStore::with_clock(clock.clone()),
        clock,
    ))
}

resource_status_conformance_suite!(store());
