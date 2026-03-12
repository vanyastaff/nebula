/// Test helpers and utilities for eventbus integration tests.

pub fn init_log() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestEvent {
    pub id: u64,
}
