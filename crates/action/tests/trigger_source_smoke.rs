//! Smoke tests for [`TriggerSource`] — verifies the trait exists,
//! is `Send + Sync + 'static`, and exposes an `Event` associated type.

use nebula_action::TriggerSource;

#[derive(Debug)]
struct DummySource;

impl TriggerSource for DummySource {
    type Event = String;
}

#[test]
fn trigger_source_compiles_with_send_sync_static_event() {
    fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<DummySource>();
    assert_send_sync_static::<<DummySource as TriggerSource>::Event>();
}
