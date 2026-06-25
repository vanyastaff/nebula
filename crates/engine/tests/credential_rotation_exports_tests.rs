#![cfg(feature = "rotation")]

//! Smoke test: the credential rotation scheduler types construct from their
//! canonical homes in `nebula-credential`. The engine no longer re-exports them
//! (ADR-0092 step 8 dropped the `credential::rotation` drain shim); this guards
//! that the engine's `rotation` feature still wires the dependency through so
//! the canonical paths remain reachable from an engine-tier consumer.

use std::time::Duration;

use nebula_credential::rotation::policy::PeriodicConfig;
use nebula_credential::runtime::rotation::PeriodicScheduler;

#[test]
fn periodic_scheduler_constructs_from_canonical_paths() {
    let config = PeriodicConfig::new(Duration::from_hours(2160), Duration::from_hours(168), false)
        .expect("valid periodic config");

    let scheduler = PeriodicScheduler::new(config);
    let _next = scheduler.schedule_rotation();
}
