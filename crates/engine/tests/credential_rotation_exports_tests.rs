#![cfg(feature = "rotation")]

//! Smoke test for engine credential rotation surface.

use std::time::Duration;

#[test]
fn periodic_scheduler_is_exposed_via_engine_credential_rotation() {
    let config = nebula_engine::credential::rotation::PeriodicConfig::new(
        Duration::from_secs(90 * 24 * 3600),
        Duration::from_secs(7 * 24 * 3600),
        false,
    )
    .expect("valid periodic config");

    let scheduler = nebula_engine::credential::rotation::PeriodicScheduler::new(config);
    let _next = scheduler.schedule_rotation();
}
