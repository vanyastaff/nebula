//! Edge-case tests for `PollingWatcher`.
//!
//! Covers lifecycle state transitions and real file-change detection using a
//! short polling interval so tests remain fast.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nebula_config::{ConfigSource, ConfigWatchEventType, ConfigWatcher, PollingWatcher};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build a `PollingWatcher` with a 50 ms interval that appends each received
/// event type to the shared `events` list.
fn make_watcher(events: Arc<Mutex<Vec<ConfigWatchEventType>>>) -> PollingWatcher {
    PollingWatcher::new(Duration::from_millis(50), move |evt| {
        tracing::debug!(event_type = ?evt.event_type, "polling watcher received event");
        events.lock().unwrap().push(evt.event_type);
    })
}

// ── Lifecycle ────────────────────────────────────────────────────────────────

/// A second `start_watching` call while the watcher is already running must
/// return an error without panicking.
#[tokio::test]
async fn already_watching_returns_err() {
    common::init_tracing();
    tracing::debug!("test: already_watching_returns_err");

    let path = common::write_temp_file("poll_double_start", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());

    let watcher = make_watcher(Arc::new(Mutex::new(Vec::new())));
    watcher
        .start_watching(&[source.clone()])
        .await
        .expect("first start_watching should succeed");

    let second = watcher.start_watching(&[source]).await;
    assert!(second.is_err(), "second start_watching must return Err");

    watcher
        .stop_watching()
        .await
        .expect("stop should succeed after double-start test");
    let _ = std::fs::remove_file(&path);
}

/// Calling `stop_watching` when the watcher is not running must return `Ok`
/// (idempotent no-op).
#[tokio::test]
async fn stop_when_not_watching_is_ok() {
    common::init_tracing();
    tracing::debug!("test: stop_when_not_watching_is_ok");

    let watcher = make_watcher(Arc::new(Mutex::new(Vec::new())));
    let result = watcher.stop_watching().await;
    assert!(
        result.is_ok(),
        "stop_watching on an idle watcher must return Ok"
    );
}

/// `is_watching` must reflect the actual watching state at each lifecycle step.
#[tokio::test]
async fn is_watching_lifecycle() {
    common::init_tracing();
    tracing::debug!("test: is_watching_lifecycle");

    let path = common::write_temp_file("poll_lifecycle", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());

    let watcher = make_watcher(Arc::new(Mutex::new(Vec::new())));

    assert!(
        !watcher.is_watching(),
        "should not be watching before start"
    );

    watcher
        .start_watching(&[source])
        .await
        .expect("start_watching should succeed");
    assert!(watcher.is_watching(), "should be watching after start");

    watcher
        .stop_watching()
        .await
        .expect("stop_watching should succeed");
    assert!(!watcher.is_watching(), "should not be watching after stop");

    let _ = std::fs::remove_file(&path);
}

// ── Change detection ─────────────────────────────────────────────────────────

/// Writing new content (different byte-length) to a watched file must trigger
/// a `Modified` event within a few polling intervals.
#[tokio::test]
async fn detects_file_modification() {
    common::init_tracing();
    tracing::debug!("test: detects_file_modification");

    let events: Arc<Mutex<Vec<ConfigWatchEventType>>> = Arc::new(Mutex::new(Vec::new()));
    let path = common::write_temp_file("poll_detect", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());

    let watcher = make_watcher(Arc::clone(&events));
    watcher
        .start_watching(&[source])
        .await
        .expect("start_watching should succeed");

    // Allow the initial scan to populate the metadata cache.
    tokio::time::sleep(Duration::from_millis(120)).await;

    // Write substantially different content so the size change is unambiguous.
    std::fs::write(&path, "x = 1\ny = 2\nz = 3\nextra = \"padding\"\n")
        .expect("should overwrite watched file");
    tracing::debug!("wrote new content to watched file");

    // Wait for several poll cycles to pick up the change.
    tokio::time::sleep(Duration::from_millis(300)).await;

    watcher.stop_watching().await.expect("stop should succeed");

    let received = events.lock().unwrap();
    tracing::debug!(?received, "events received after modification");
    assert!(
        received
            .iter()
            .any(|e| *e == ConfigWatchEventType::Modified),
        "expected at least one Modified event; got: {received:?}"
    );

    let _ = std::fs::remove_file(&path);
}

/// Creating a file that did not exist when watching started must trigger a
/// `Created` event.
#[tokio::test]
async fn detects_file_creation() {
    common::init_tracing();
    tracing::debug!("test: detects_file_creation");

    // Build a path that does NOT exist yet.
    let dir = std::env::temp_dir();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = dir.join(format!("nebula_config_edge_poll_create_{ts}.toml"));
    let source = ConfigSource::File(path.clone());

    let events: Arc<Mutex<Vec<ConfigWatchEventType>>> = Arc::new(Mutex::new(Vec::new()));
    let watcher = make_watcher(Arc::clone(&events));
    watcher
        .start_watching(&[source])
        .await
        .expect("start_watching should succeed for non-existent path");

    // Let the initial scan run (it will find no file, leaving cache empty).
    tokio::time::sleep(Duration::from_millis(120)).await;

    // Now create the file.
    std::fs::write(&path, "created = true\n").expect("should create watched file");
    tracing::debug!("created watched file");

    // Wait for detection.
    tokio::time::sleep(Duration::from_millis(300)).await;

    watcher.stop_watching().await.expect("stop should succeed");

    let received = events.lock().unwrap();
    tracing::debug!(?received, "events received after creation");
    assert!(
        received.iter().any(|e| *e == ConfigWatchEventType::Created),
        "expected at least one Created event; got: {received:?}"
    );

    let _ = std::fs::remove_file(&path);
}
