//! Edge-case tests for `FileWatcher`.
//!
//! Focuses on lifecycle correctness (start / stop idempotency, `is_watching`
//! invariants) and absence of panics after the watcher has been stopped.

mod common;

use std::time::Duration;

use nebula_config::{ConfigSource, ConfigWatcher, FileWatcher};
use tokio_util::sync::CancellationToken;

// ── Lifecycle ────────────────────────────────────────────────────────────────

/// `stop_watching` on an active `FileWatcher` must return `Ok`.
#[tokio::test]
async fn stop_watching_returns_ok() {
    common::init_tracing();
    tracing::debug!("test: stop_watching_returns_ok");

    let path = common::write_temp_file("fw_stop_ok", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());

    let watcher = FileWatcher::new(|_| {});
    watcher
        .start_watching(&[source], CancellationToken::new())
        .await
        .expect("start_watching should succeed");

    let result = watcher.stop_watching().await;
    assert!(
        result.is_ok(),
        "stop_watching must return Ok on an active watcher"
    );

    let _ = std::fs::remove_file(&path);
}

/// `is_watching` must return `false` after `stop_watching` completes.
#[tokio::test]
async fn is_watching_false_after_stop() {
    common::init_tracing();
    tracing::debug!("test: is_watching_false_after_stop");

    let path = common::write_temp_file("fw_is_watching", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());

    let watcher = FileWatcher::new(|_| {});
    watcher
        .start_watching(&[source], CancellationToken::new())
        .await
        .expect("start_watching should succeed");
    assert!(watcher.is_watching(), "should be watching after start");

    watcher.stop_watching().await.expect("stop should succeed");
    assert!(
        !watcher.is_watching(),
        "is_watching must be false after stop"
    );

    let _ = std::fs::remove_file(&path);
}

/// A second `start_watching` while the watcher is already active must return
/// an `Err` without panicking.
#[tokio::test]
async fn already_watching_returns_err() {
    common::init_tracing();
    tracing::debug!("test: already_watching_returns_err");

    let path = common::write_temp_file("fw_double_start", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());

    let watcher = FileWatcher::new(|_| {});
    let cancel = CancellationToken::new();
    watcher
        .start_watching(std::slice::from_ref(&source), cancel.clone())
        .await
        .expect("first start_watching should succeed");

    let second = watcher.start_watching(&[source], cancel).await;
    assert!(second.is_err(), "second start_watching must return Err");

    watcher
        .stop_watching()
        .await
        .expect("stop should succeed after double-start test");
    let _ = std::fs::remove_file(&path);
}

/// Writing to a file after `stop_watching` must not cause a panic or any
/// observable error in the test process.  The debounce / event-processor task
/// should have been dropped and must not surface again.
#[tokio::test]
async fn no_panic_on_write_after_stop() {
    common::init_tracing();
    tracing::debug!("test: no_panic_on_write_after_stop");

    let path = common::write_temp_file("fw_no_panic", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());

    let watcher = FileWatcher::new(|_| {});
    watcher
        .start_watching(&[source], CancellationToken::new())
        .await
        .expect("start_watching should succeed");
    watcher.stop_watching().await.expect("stop should succeed");

    tracing::debug!("watcher stopped; writing to file to verify no panic");

    // Writes after stop must not panic.
    std::fs::write(&path, "x = 99\ny = \"post-stop\"\n")
        .expect("write should succeed regardless of watcher state");

    // Give any lingering background tasks a chance to observe the write.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Reaching here without a panic is sufficient.
    let _ = std::fs::remove_file(&path);
}

/// `stop_watching` on a watcher that was never started must return `Ok`
/// (idempotent no-op mirrors `PollingWatcher` behaviour).
#[tokio::test]
async fn stop_when_not_watching_is_ok() {
    common::init_tracing();
    tracing::debug!("test: stop_when_not_watching_is_ok");

    let watcher = FileWatcher::new(|_| {});
    let result = watcher.stop_watching().await;
    assert!(
        result.is_ok(),
        "stop_watching on idle FileWatcher must return Ok"
    );
}
