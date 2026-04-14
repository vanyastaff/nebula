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

// ── #294: CAS uniqueness under concurrent `start_watching` ───────────────────

/// Hammering `start_watching` from many tasks concurrently must produce
/// **exactly one** successful claim. Before #294 the load/store pair raced and
/// could spawn duplicate notify handles.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_start_watching_exactly_one_succeeds() {
    common::init_tracing();
    tracing::debug!("test: concurrent_start_watching_exactly_one_succeeds");

    let path = common::write_temp_file("fw_cas_unique", "toml", "x = 1\n");
    let source = ConfigSource::File(path.clone());

    let watcher = std::sync::Arc::new(FileWatcher::new(|_| {}));
    let cancel = CancellationToken::new();

    // Spawn 16 concurrent attempts. Use a shared barrier-ish setup via
    // `tokio::join!` is awkward at this fan-out, so use JoinSet instead.
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let w = std::sync::Arc::clone(&watcher);
        let s = source.clone();
        let c = cancel.clone();
        set.spawn(async move { w.start_watching(&[s], c).await });
    }

    let mut ok = 0usize;
    let mut already = 0usize;
    while let Some(joined) = set.join_next().await {
        match joined.expect("task must not panic") {
            Ok(()) => ok += 1,
            Err(e) if e.to_string().contains("Already watching") => already += 1,
            Err(other) => panic!("unexpected error from start_watching: {other}"),
        }
    }

    assert_eq!(
        ok, 1,
        "exactly one start_watching must succeed (CAS claim); got {ok} ok, {already} already-watching"
    );
    assert_eq!(
        already, 15,
        "the remaining 15 calls must report Already watching"
    );
    assert!(watcher.is_watching(), "watcher must be in watching state");

    watcher.stop_watching().await.expect("stop should succeed");
    let _ = std::fs::remove_file(&path);
}

/// If setup fails after the CAS claim (e.g. notify cannot watch the requested
/// path), `start_watching` must release the claim so a subsequent retry can
/// succeed. Without the unwind guard the watcher would be wedged in
/// `is_watching == true` for the rest of its life.
#[tokio::test]
async fn claim_unwinds_on_setup_failure() {
    common::init_tracing();
    tracing::debug!("test: claim_unwinds_on_setup_failure");

    // Pick a definitely-non-existent directory so notify's `watch()` errors.
    // Use a unique suffix to avoid collisions across parallel test runs.
    let bogus = std::env::temp_dir().join(format!(
        "nebula_config_definitely_not_a_dir_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bogus);

    // Create the file path, but do NOT create the parent directory. notify
    // resolves the parent and fails — the CAS unwind path must release the
    // claim. (We pre-create no file, so `path.is_file()` is false and notify
    // tries to watch the bogus parent directly.)
    let bogus_file = bogus.join("config.toml");
    let source = ConfigSource::File(bogus_file);

    let watcher = FileWatcher::new(|_| {});

    let first = watcher
        .start_watching(&[source.clone()], CancellationToken::new())
        .await;
    assert!(
        first.is_err(),
        "start_watching against a non-existent parent must return Err"
    );
    assert!(
        !watcher.is_watching(),
        "claim must have been released after setup failure (#294 unwind guard)"
    );

    // A retry against a real path must now succeed — proves the claim slot
    // is actually free, not just reported as such.
    let real_path = common::write_temp_file("fw_unwind_retry", "toml", "x = 1\n");
    let retry = watcher
        .start_watching(
            &[ConfigSource::File(real_path.clone())],
            CancellationToken::new(),
        )
        .await;
    assert!(
        retry.is_ok(),
        "retry after unwind must succeed; got {retry:?}"
    );

    watcher.stop_watching().await.expect("stop should succeed");
    let _ = std::fs::remove_file(&real_path);
}

// ── #310: notify callback must be non-blocking under burst ───────────────────

/// The notify callback must never block the OS notifier thread. With the
/// pre-#310 `blocking_send`, a burst of writes that overflows the forwarding
/// channel could stall the kernel notifier indefinitely. Post-fix, the
/// callback uses `try_send`; the burst completes promptly, and the watcher
/// continues to receive events without the test process deadlocking.
///
/// The previous test variant tried to assert that a "sentinel" write after
/// the burst was observed within a deadline — but notify backends vary
/// wildly across platforms in how they coalesce / debounce events under
/// pressure, and on Windows in particular a sentinel can be silently
/// merged into the burst. We instead assert the only property that
/// actually proves the callback did not block: the synchronous writer
/// pushed the entire burst to completion within a bounded wall-clock
/// budget, and the watcher delivered at least *some* events through the
/// non-blocking forwarder.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn burst_events_do_not_block_notifier() {
    common::init_tracing();
    tracing::debug!("test: burst_events_do_not_block_notifier");

    // Use a directory so a single watch covers every burst write at once,
    // and the notify backend tends to coalesce less aggressively.
    let dir = std::env::temp_dir().join(format!(
        "nebula_config_burst_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir must be creatable");

    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter_cb = std::sync::Arc::clone(&counter);
    let watcher = std::sync::Arc::new(FileWatcher::new(move |_event| {
        counter_cb.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }));

    watcher
        .start_watching(
            &[ConfigSource::Directory(dir.clone())],
            CancellationToken::new(),
        )
        .await
        .expect("start should succeed");

    // Hammer the directory with many small writes in rapid succession.
    let burst = 2_000usize;
    let burst_started = std::time::Instant::now();
    for i in 0..burst {
        let p = dir.join(format!("burst_{i}.toml"));
        std::fs::write(&p, format!("idx = {i}\n")).expect("write must succeed");
    }
    let burst_elapsed = burst_started.elapsed();

    // Bounded wait for the in-process forwarder to drain whatever events
    // the OS surfaced. The exact post-burst count is platform-dependent;
    // we just need *something* to demonstrate the callback wasn't blocked
    // on `blocking_send` (the pre-#310 behaviour).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline
        && counter.load(std::sync::atomic::Ordering::Relaxed) == 0
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let dropped = watcher.dropped_events();
    let observed = counter.load(std::sync::atomic::Ordering::Relaxed);

    tracing::debug!(
        burst = burst,
        observed = observed,
        dropped = dropped,
        burst_elapsed_ms = burst_elapsed.as_millis() as u64,
        "burst test summary"
    );

    // Primary assertion: the synchronous burst completed in bounded time.
    // Pre-#310, a `blocking_send` into a saturated channel could push this
    // toward `Duration::MAX` if the forwarder task fell behind. Allow a
    // generous bound to keep the test robust on slow CI: 2000 small writes
    // should complete well inside a few seconds even on the slowest runner.
    assert!(
        burst_elapsed < Duration::from_secs(15),
        "synchronous burst took {:?} — the notifier callback may be blocking",
        burst_elapsed
    );

    // Secondary assertion: the watcher actually delivered events through
    // the non-blocking forwarder. Without this, we'd be unable to
    // distinguish a working pipeline from a wedged one.
    assert!(
        observed > 0,
        "no events observed at all — non-blocking forwarder may have dropped everything"
    );

    // `dropped_events()` is the new observability surface. Regardless of
    // whether this run actually hit overflow (notify coalesces enough to
    // fit under the cap on most machines), the accessor must exist and
    // return a sane u64.
    let _ = dropped;

    watcher.stop_watching().await.expect("stop should succeed");
    let _ = std::fs::remove_dir_all(&dir);
}
