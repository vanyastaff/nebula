//! Polling-based watcher for environments without native file watching

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::{
    core::{ConfigError, ConfigResult, ConfigSource, ConfigWatcher},
    watchers::{ConfigWatchEvent, ConfigWatchEventType},
};

/// Polling watcher that checks for changes at regular intervals
pub struct PollingWatcher {
    /// Polling interval
    interval: std::time::Duration,

    /// Callback for configuration changes
    callback: Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,

    /// Currently watching
    watching: Arc<AtomicBool>,

    /// Task handle for the polling loop
    task_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,

    /// File metadata cache
    metadata_cache: Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
}

#[derive(Debug, Clone, PartialEq)]
struct FileMetadata {
    modified: std::time::SystemTime,
    size: u64,
}

impl std::fmt::Debug for PollingWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollingWatcher")
            .field("interval", &self.interval)
            .field("watching", &self.watching.load(Ordering::Relaxed))
            .finish()
    }
}

impl PollingWatcher {
    /// Create a new polling watcher
    pub fn new<F>(interval: std::time::Duration, callback: F) -> Self
    where
        F: Fn(ConfigWatchEvent) + Send + Sync + 'static,
    {
        Self {
            interval,
            callback: Arc::new(callback),
            watching: Arc::new(AtomicBool::new(false)),
            task_handle: Arc::new(RwLock::new(None)),
            metadata_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create with default interval (5 seconds)
    pub fn with_callback<F>(callback: F) -> Self
    where
        F: Fn(ConfigWatchEvent) + Send + Sync + 'static,
    {
        Self::new(std::time::Duration::from_secs(5), callback)
    }

    /// Create a new polling watcher with no-op callback
    pub fn new_noop(interval: std::time::Duration) -> Self {
        Self::new(interval, |_| {})
    }

    /// Set polling interval
    #[must_use = "builder methods must be chained or built"]
    pub fn with_interval(mut self, interval: std::time::Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Get file metadata
    async fn get_file_metadata(path: &PathBuf) -> Option<FileMetadata> {
        match tokio::fs::metadata(path).await {
            Ok(metadata) => Some(FileMetadata {
                modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                size: metadata.len(),
            }),
            Err(_) => None,
        }
    }

    /// Check if file has changed
    fn has_changed(old: &FileMetadata, new: &FileMetadata) -> bool {
        old.modified != new.modified || old.size != new.size
    }

    /// Start the polling loop.
    ///
    /// The loop exits on either:
    /// - `cancel.cancelled()` — fired by `Config::drop` or `stop_watching`,
    /// - `watching` flipped to `false` — legacy shutdown path for callers that still call
    ///   `stop_watching` explicitly.
    ///
    /// The `watching` flag is cleared on exit regardless of which path
    /// triggered shutdown, so `is_watching()` always reflects the real state
    /// after the spawned task has observed cancellation.
    #[allow(clippy::excessive_nesting)] // Reason: polling loop with per-source type dispatch
    async fn start_polling_loop(
        &self,
        sources: Vec<ConfigSource>,
        callback: Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
        watching: Arc<AtomicBool>,
        metadata_cache: Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
        interval: std::time::Duration,
        cancel: CancellationToken,
    ) {
        let mut interval_timer = tokio::time::interval(interval);
        interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Initial scan to populate cache (collect all metadata first, then lock once).
        // Racing against cancel during the initial scan lets `Config::drop` during
        // a slow startup scan still tear the task down promptly.
        let mut initial_entries = Vec::new();
        for source in &sources {
            if cancel.is_cancelled() {
                watching.store(false, Ordering::Release);
                return;
            }
            match source {
                ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                    if let Some(metadata) = Self::get_file_metadata(path).await {
                        initial_entries.push((path.clone(), metadata));
                    }
                },
                ConfigSource::Directory(dir) => {
                    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            let path = entry.path();
                            if let Some(metadata) = Self::get_file_metadata(&path).await {
                                initial_entries.push((path, metadata));
                            }
                        }
                    }
                },
                _ => {},
            }
        }
        if !initial_entries.is_empty() {
            let mut cache = metadata_cache.write().await;
            for (path, metadata) in initial_entries {
                cache.insert(path, metadata);
            }
        }

        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                _ = interval_timer.tick() => {}
            }

            // The legacy `stop_watching` path flips this flag and then waits
            // for the task to exit at its next iteration. Honor it here too
            // so both shutdown paths behave identically.
            if !watching.load(Ordering::Acquire) {
                break;
            }

            // Check all sources concurrently per tick
            type BoxFut<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
            let check_futures: Vec<BoxFut<'_>> = sources
                .iter()
                .filter_map(|source| -> Option<BoxFut<'_>> {
                    match source {
                        ConfigSource::File(path) | ConfigSource::FileAuto(path) => Some(Box::pin(
                            self.check_file_changes(path, source, &callback, &metadata_cache),
                        )),
                        ConfigSource::Directory(dir) => Some(Box::pin(
                            self.check_directory_changes(dir, source, &callback, &metadata_cache),
                        )),
                        _ => None,
                    }
                })
                .collect();
            futures::future::join_all(check_futures).await;
        }

        // Clear the status mirror so `is_watching()` becomes correct even
        // when cancellation (not `stop_watching`) was the trigger.
        watching.store(false, Ordering::Release);
    }

    /// Check for file changes (read lock first, write lock only on change)
    async fn check_file_changes(
        &self,
        path: &PathBuf,
        source: &ConfigSource,
        callback: &Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
        cache: &Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
    ) {
        let current_metadata = Self::get_file_metadata(path).await;

        // Read lock to detect if change occurred
        let change = {
            let cache_read = cache.read().await;
            match (cache_read.get(path), &current_metadata) {
                (Some(old), Some(new)) if Self::has_changed(old, new) => {
                    Some(ConfigWatchEventType::Modified)
                },
                (Some(_), None) => Some(ConfigWatchEventType::Deleted),
                (None, Some(_)) => Some(ConfigWatchEventType::Created),
                _ => None,
            }
        };

        // Write lock only when we need to update
        if let Some(event_type) = change {
            let event =
                ConfigWatchEvent::new(event_type.clone(), source.clone()).with_path(path.clone());
            callback(event);

            let mut cache_write = cache.write().await;
            match event_type {
                ConfigWatchEventType::Deleted => {
                    cache_write.remove(path);
                },
                _ => {
                    if let Some(metadata) = current_metadata {
                        cache_write.insert(path.clone(), metadata);
                    }
                },
            }
        }
    }

    /// Check for directory changes (scan before locking, minimize lock scope)
    #[allow(clippy::excessive_nesting)] // Reason: directory traversal with file comparison logic
    async fn check_directory_changes(
        &self,
        dir: &PathBuf,
        source: &ConfigSource,
        callback: &Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
        cache: &Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
    ) {
        // Collect directory entries, then fetch all metadata concurrently
        let mut paths = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                paths.push(entry.path());
            }
        }
        let metadata_results =
            futures::future::join_all(paths.iter().map(Self::get_file_metadata)).await;
        let mut current_files: HashMap<_, _> = paths
            .into_iter()
            .zip(metadata_results)
            .filter_map(|(path, meta)| meta.map(|m| (path, m)))
            .collect();

        // Collect events under read lock
        let events: Vec<(PathBuf, ConfigWatchEventType)> = {
            let cache_read = cache.read().await;

            let mut events = Vec::new();

            // Check for modifications and creations
            for (path, new_metadata) in &current_files {
                if !path.starts_with(dir) {
                    continue;
                }
                match cache_read.get(path) {
                    Some(old) if Self::has_changed(old, new_metadata) => {
                        events.push((path.clone(), ConfigWatchEventType::Modified));
                    },
                    None => {
                        events.push((path.clone(), ConfigWatchEventType::Created));
                    },
                    _ => {},
                }
            }

            // Check for deletions
            for path in cache_read.keys() {
                if path.starts_with(dir) && !current_files.contains_key(path) {
                    events.push((path.clone(), ConfigWatchEventType::Deleted));
                }
            }

            events
        };

        // Fire callbacks outside any lock
        for (path, event_type) in &events {
            let event =
                ConfigWatchEvent::new(event_type.clone(), source.clone()).with_path(path.clone());
            callback(event);
        }

        // Update cache under write lock only if there were changes
        if !events.is_empty() {
            let mut cache_write = cache.write().await;
            for (path, event_type) in events {
                match event_type {
                    ConfigWatchEventType::Deleted => {
                        cache_write.remove(&path);
                    },
                    _ => {
                        if let Some(metadata) = current_files.remove(&path) {
                            cache_write.insert(path, metadata);
                        }
                    },
                }
            }
        }
    }
}

#[async_trait]
impl ConfigWatcher for PollingWatcher {
    async fn start_watching(
        &self,
        sources: &[ConfigSource],
        cancel: CancellationToken,
    ) -> ConfigResult<()> {
        // Atomically claim the watching slot. Using compare_exchange closes
        // the race window where two concurrent `start_watching` calls could
        // both pass a load/store pair and leak one of the spawned tasks.
        if self
            .watching
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ConfigError::watch_error("Already watching"));
        }

        let sources = sources.to_vec();
        let callback = Arc::clone(&self.callback);
        let watching = Arc::clone(&self.watching);
        let metadata_cache = Arc::clone(&self.metadata_cache);
        let interval = self.interval;

        // Clear the cache
        {
            let mut cache = metadata_cache.write().await;
            cache.clear();
        }

        // Start polling task. The cancel token is owned by the spawned loop;
        // `Config::drop` (sync) cancels the parent and this task exits on its
        // next select! tick. This is the #315 fix: without the token the task
        // leaked when the owning `Config` was dropped.
        let watcher = self.clone();
        let handle = tokio::spawn(async move {
            watcher
                .start_polling_loop(
                    sources,
                    callback,
                    watching,
                    metadata_cache,
                    interval,
                    cancel,
                )
                .await;
        });

        // Store task handle
        {
            let mut task_handle = self.task_handle.write().await;
            *task_handle = Some(handle);
        }

        nebula_log::info!("Started polling watcher with interval {:?}", self.interval);

        Ok(())
    }

    async fn stop_watching(&self) -> ConfigResult<()> {
        // Check if actually watching
        if !self.watching.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Mark as not watching
        self.watching.store(false, Ordering::Relaxed);

        // Wait for the polling task to exit (it checks `watching` flag each tick)
        let handle = {
            let mut task_handle = self.task_handle.write().await;
            task_handle.take()
        };
        if let Some(handle) = handle {
            // The task will exit on next interval tick when it sees watching == false.
            // Use a timeout to avoid waiting indefinitely if the task is stuck;
            // on timeout, abort explicitly so the runtime reclaims the task
            // rather than leaving a zombie behind.
            let abort_handle = handle.abort_handle();
            if tokio::time::timeout(self.interval * 2, handle)
                .await
                .is_err()
            {
                nebula_log::warn!("Polling task did not exit within timeout; aborting");
                abort_handle.abort();
            }
        }

        // Clear metadata cache
        {
            let mut cache = self.metadata_cache.write().await;
            cache.clear();
        }

        nebula_log::info!("Stopped polling watcher");

        Ok(())
    }

    fn is_watching(&self) -> bool {
        self.watching.load(Ordering::Relaxed)
    }
}

// Implement Clone manually since we have Arc fields
impl Clone for PollingWatcher {
    fn clone(&self) -> Self {
        Self {
            interval: self.interval,
            callback: Arc::clone(&self.callback),
            watching: Arc::clone(&self.watching),
            task_handle: Arc::clone(&self.task_handle),
            metadata_cache: Arc::clone(&self.metadata_cache),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #315: Before the fix, `PollingWatcher`'s loop only exited on
    /// `stop_watching().await`, which `Config::drop` (sync) cannot call.
    /// Cancelling the token must now terminate the spawned task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn polling_task_exits_on_token_cancel() {
        let watcher = PollingWatcher::new_noop(std::time::Duration::from_millis(20));
        let cancel = CancellationToken::new();

        watcher
            .start_watching(&[ConfigSource::Env], cancel.clone())
            .await
            .expect("start_watching must succeed");
        assert!(watcher.is_watching());

        // Fire the parent signal `Config::drop` would fire, and let the
        // task observe it on its next select! tick.
        cancel.cancel();

        // The loop rechecks on each tick; give it up to ~10 ticks to exit.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while watcher.is_watching() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            !watcher.is_watching(),
            "watcher did not clear is_watching() after cancel"
        );

        // Task handle must be joinable and finished.
        let handle = watcher
            .task_handle
            .write()
            .await
            .take()
            .expect("task handle must exist");
        assert!(
            handle.is_finished(),
            "spawned polling task must have exited after cancel"
        );
        if let Err(e) = handle.await {
            assert!(!e.is_panic(), "polling task panicked: {e}");
        }
    }

    /// Regression for the pre-fix behavior: an explicit `stop_watching()`
    /// still tears the task down cleanly without relying on the cancel path.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn polling_task_exits_on_explicit_stop() {
        let watcher = PollingWatcher::new_noop(std::time::Duration::from_millis(20));
        let cancel = CancellationToken::new();
        watcher
            .start_watching(&[ConfigSource::Env], cancel)
            .await
            .expect("start_watching must succeed");
        assert!(watcher.is_watching());

        watcher
            .stop_watching()
            .await
            .expect("stop_watching must succeed");
        assert!(!watcher.is_watching());
    }

    /// Double-start must return `Err(Already watching)` via the `compare_exchange`
    /// guard, not leak a second spawned task.
    #[tokio::test]
    async fn double_start_watching_is_rejected() {
        let watcher = PollingWatcher::new_noop(std::time::Duration::from_millis(50));
        let cancel = CancellationToken::new();

        watcher
            .start_watching(&[ConfigSource::Env], cancel.clone())
            .await
            .expect("first start must succeed");

        let err = watcher
            .start_watching(&[ConfigSource::Env], cancel.clone())
            .await
            .expect_err("second start must fail");
        assert!(err.to_string().contains("Already watching"));

        // Clean up so the test doesn't leave a background task around.
        cancel.cancel();
    }
}
