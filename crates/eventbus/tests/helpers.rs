//! Test helpers and utilities for eventbus integration tests.
//!
//! Integration-test binaries are compiled independently, so Rust cannot see
//! that `integration.rs` uses these items via `mod helpers`. Silence the
//! resulting `dead_code` false positive here.

#![allow(dead_code, reason = "consumed by sibling integration-test binaries")]

pub(crate) fn init_log() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestEvent {
    pub id: u64,
}
