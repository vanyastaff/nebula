//! Edge-case tests for `FileWatcher`.
//!
//! Focuses on lifecycle correctness (start / stop idempotency, `is_watching`
//! invariants) and absence of panics after the watcher has been stopped.

mod common;

use std::{sync::Arc, time::Duration};

use nebula_config::{ConfigSource, ConfigWatcher, FileWatcher};

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
        .start_watching(&[source])
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
        .start_watching(&[source])
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
    watcher
        .start_watching(std::slice::from_ref(&source))
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

/// Concurrent `start_watching` calls must claim the watcher atomically:
/// exactly one call succeeds and all others return `Already watching`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_start_watching_allows_only_single_winner() {
    common::init_tracing();
    tracing::debug!("test: concurrent_start_watching_allows_only_single_winner");

    let path = common::write_temp_file("fw_concurrent_start", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());
    let watcher = Arc::new(FileWatcher::new(|_| {}));

    let attempts = 8usize;
    let mut tasks = Vec::with_capacity(attempts);

    for _ in 0..attempts {
        let watcher = Arc::clone(&watcher);
        let source = source.clone();
        tasks.push(tokio::spawn(async move {
            watcher.start_watching(&[source]).await
        }));
    }

    let results = futures::future::join_all(tasks).await;
    let successes = results
        .iter()
        .filter(|res| matches!(res, Ok(Ok(()))))
        .count();
    let already_watching_errors = results
        .iter()
        .filter(|res| match res {
            Ok(Err(err)) => err.to_string().contains("Already watching"),
            _ => false,
        })
        .count();

    assert_eq!(successes, 1, "exactly one concurrent start must succeed");
    assert_eq!(
        already_watching_errors,
        attempts - 1,
        "all other concurrent starts must return `Already watching`"
    );

    watcher
        .stop_watching()
        .await
        .expect("stop should succeed after concurrent-start test");
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
        .start_watching(&[source])
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
