//! File system watcher for configuration files

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

/// Capacity of the in-process channel that forwards `notify` filesystem events
/// to the debouncing event-processor task. Sized to absorb normal deploy-storm
/// bursts; when bursts exceed this depth, events are dropped non-blockingly
/// and the count is exposed via [`FileWatcher::dropped_events`] (#310).
const FORWARD_CHANNEL_CAPACITY: usize = 512;

/// Non-blocking forward of a `ConfigWatchEvent` from the notify callback
/// thread into the debounce/processor channel.
///
/// Using `try_send` is mandatory: the notify callback runs on the OS notifier
/// thread, and a blocking send under burst load can stall the kernel notifier
/// (#310). When the channel is full, the event is dropped, the counter is
/// incremented, and a WARN is emitted at power-of-two intervals so dashboards
/// can detect saturation without spamming the log.
fn forward_event(
    tx: &mpsc::Sender<ConfigWatchEvent>,
    event: ConfigWatchEvent,
    dropped: &Arc<AtomicU64>,
) {
    match tx.try_send(event) {
        Ok(()) => {},
        Err(mpsc::error::TrySendError::Full(_)) => {
            let n = dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_power_of_two() {
                nebula_log::warn!(
                    dropped_total = n,
                    "FileWatcher forwarding channel full; dropping fs event"
                );
            }
        },
        Err(mpsc::error::TrySendError::Closed(_)) => {
            // Forward task already exited — benign during shutdown.
        },
    }
}

use crate::{
    core::{ConfigError, ConfigResult, ConfigSource},
    watchers::{ConfigWatchEvent, ConfigWatchEventType},
};

/// File system watcher for configuration files
pub struct FileWatcher {
    /// File system watcher instance
    watcher: Arc<RwLock<Option<RecommendedWatcher>>>,

    /// Event sender
    event_tx: Arc<RwLock<Option<mpsc::Sender<ConfigWatchEvent>>>>,

    /// Callback for configuration changes
    callback: Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,

    /// Currently watching
    watching: Arc<AtomicBool>,

    /// Map of paths to sources for event routing
    path_to_source: Arc<RwLock<HashMap<PathBuf, ConfigSource>>>,

    /// Debounce duration (to avoid multiple events for same change)
    debounce_duration: std::time::Duration,

    /// Count of `notify` events dropped because the forwarding channel was
    /// full at delivery time. Exposed via [`FileWatcher::dropped_events`] for
    /// dashboards and tests. The notifier thread must never block on send
    /// (#310): losing a filesystem event is preferable to stalling the kernel
    /// notifier — the next file change will retrigger anyway.
    dropped_events: Arc<AtomicU64>,
}

impl std::fmt::Debug for FileWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWatcher")
            .field("watching", &self.watching.load(Ordering::Relaxed))
            .field("debounce_duration", &self.debounce_duration)
            .finish()
    }
}

impl FileWatcher {
    /// Create a new file watcher
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(ConfigWatchEvent) + Send + Sync + 'static,
    {
        Self {
            watcher: Arc::new(RwLock::new(None)),
            event_tx: Arc::new(RwLock::new(None)),
            callback: Arc::new(callback),
            watching: Arc::new(AtomicBool::new(false)),
            path_to_source: Arc::new(RwLock::new(HashMap::new())),
            debounce_duration: std::time::Duration::from_millis(100),
            dropped_events: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Number of filesystem events dropped due to forwarding-channel
    /// saturation since this watcher was created.
    ///
    /// Exposed for observability / tests. Increments are produced by the
    /// notify callback when its `try_send` returns `Full` — see #310.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events.load(Ordering::Relaxed)
    }

    /// Create a new file watcher with no-op callback
    pub fn new_noop() -> Self {
        Self::new(|_| {})
    }

    /// Set debounce duration
    #[must_use = "builder methods must be chained or built"]
    pub fn with_debounce(mut self, duration: std::time::Duration) -> Self {
        self.debounce_duration = duration;
        self
    }

    /// Start the event processing task.
    ///
    /// The task exits when either the event channel is closed *or* `cancel`
    /// fires. The cancel path is essential for #315: `Config::drop` is sync
    /// and cannot `stop_watching().await`, so cancellation is the only way
    /// to guarantee the spawned task is reclaimed when the owner drops.
    #[allow(clippy::excessive_nesting)] // Reason: async event loop with debounce logic
    async fn start_event_processor(
        &self,
        mut rx: mpsc::Receiver<ConfigWatchEvent>,
        cancel: CancellationToken,
    ) {
        let callback = Arc::clone(&self.callback);
        let debounce_duration = self.debounce_duration;
        let watching = Arc::clone(&self.watching);

        tokio::spawn(async move {
            let mut last_events: HashMap<PathBuf, std::time::Instant> = HashMap::new();

            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    maybe_event = rx.recv() => {
                        let Some(event) = maybe_event else { break; };
                        // Debounce events for the same path
                        if let Some(path) = &event.path {
                            let now = std::time::Instant::now();

                            if let Some(last_time) = last_events.get(path)
                                && now.duration_since(*last_time) < debounce_duration
                            {
                                continue; // Skip this event
                            }

                            last_events.insert(path.clone(), now);
                        }

                        // Call the callback
                        (callback)(event);
                    }
                }
            }

            // Mirror the status flag to the real exit state so external
            // `is_watching()` observers see the task is gone.
            watching.store(false, Ordering::Release);
        });
    }
}

#[async_trait]
impl crate::core::ConfigWatcher for FileWatcher {
    async fn start_watching(
        &self,
        sources: &[ConfigSource],
        cancel: CancellationToken,
    ) -> ConfigResult<()> {
        // #294: atomically claim the watching slot. Using `compare_exchange`
        // closes the race window where two concurrent `start_watching` calls
        // could both pass a load/store pair and leak one of the spawned
        // notify handles. Mirrors `PollingWatcher`'s pattern.
        if self
            .watching
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ConfigError::watch_error("Already watching"));
        }

        // RAII unwind guard: any early-return below the CAS must release the
        // claim so a retry (after fixing the underlying error) can succeed.
        // We defuse this guard at the bottom once setup is fully complete.
        struct ClaimGuard<'a>(&'a AtomicBool, bool);
        impl Drop for ClaimGuard<'_> {
            fn drop(&mut self) {
                if self.1 {
                    self.0.store(false, Ordering::Release);
                }
            }
        }
        let mut claim_guard = ClaimGuard(self.watching.as_ref(), true);

        // Create channel for events. Bounded — see FORWARD_CHANNEL_CAPACITY.
        let (tx, rx) = mpsc::channel(FORWARD_CHANNEL_CAPACITY);

        // Store the sender
        {
            let mut event_tx = self.event_tx.write().await;
            *event_tx = Some(tx.clone());
        }

        // Create path to source mapping
        let mut path_to_source = HashMap::new();
        let mut paths_to_watch = Vec::new();

        for source in sources {
            match source {
                ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                    path_to_source.insert(path.clone(), source.clone());
                    paths_to_watch.push(path.clone());
                },
                ConfigSource::Directory(dir) => {
                    path_to_source.insert(dir.clone(), source.clone());
                    paths_to_watch.push(dir.clone());
                },
                _ => {
                    // Skip non-file sources
                },
            }
        }

        if paths_to_watch.is_empty() {
            nebula_log::debug!("No file sources to watch");
            // Nothing to spawn — release the claim so a future `start_watching`
            // with real sources can proceed. The Drop on `claim_guard` handles
            // the release. Treat this as an Ok no-op.
            return Ok(());
        }

        // Store the mapping
        {
            let mut mapping = self.path_to_source.write().await;
            *mapping = path_to_source.clone();
        }

        // Create file system watcher.
        //
        // The notify callback runs on the OS notifier thread and MUST NOT
        // block — see #310. We use `try_send` and account for any drops via
        // `dropped_events`.
        let tx_clone = tx.clone();
        let path_mapping = Arc::clone(&self.path_to_source);
        let dropped = Arc::clone(&self.dropped_events);

        let mut fs_watcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        // Find the source for this path
                        if let Some(path) = event.paths.first() {
                            // Try to find exact match or parent directory
                            let mapping = path_mapping.blocking_read();

                            let source = mapping
                                .iter()
                                .find(|(watched_path, _)| {
                                    path == *watched_path || path.starts_with(watched_path)
                                })
                                .map(|(_, source)| source.clone());

                            if let Some(source) = source {
                                let event_type = match event.kind {
                                    EventKind::Create(_) => ConfigWatchEventType::Created,
                                    EventKind::Modify(_) => ConfigWatchEventType::Modified,
                                    EventKind::Remove(_) => ConfigWatchEventType::Deleted,
                                    _ => ConfigWatchEventType::Other("Unknown".to_string()),
                                };

                                let watch_event = ConfigWatchEvent::new(event_type, source)
                                    .with_path(path.clone());

                                forward_event(&tx_clone, watch_event, &dropped);
                            }
                        }
                    },
                    Err(e) => {
                        nebula_log::error!("Watch error: {}", e);

                        let error_event = ConfigWatchEvent::new(
                            ConfigWatchEventType::Error(e.to_string()),
                            ConfigSource::File(PathBuf::new()),
                        );

                        forward_event(&tx_clone, error_event, &dropped);
                    },
                }
            })
            .map_err(|e| ConfigError::watch_error(e.to_string()))?;

        // Watch all paths
        for path in paths_to_watch {
            let mode = if path.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };

            // Watch the parent directory for file changes
            let watch_path = if path.is_file() {
                path.parent().unwrap_or(&path)
            } else {
                &path
            };

            fs_watcher.watch(watch_path, mode).map_err(|e| {
                ConfigError::watch_error(format!("Failed to watch {}: {}", watch_path.display(), e))
            })?;

            nebula_log::debug!("Watching path: {}", watch_path.display());
        }

        // Store the watcher
        {
            let mut watcher = self.watcher.write().await;
            *watcher = Some(fs_watcher);
        }

        // Start event processor bound to the owner's cancel token so
        // dropping the parent `Config` tears this task down.
        self.start_event_processor(rx, cancel).await;

        // All setup succeeded — defuse the unwind guard. The `watching` flag
        // is already `true` from the CAS at entry.
        claim_guard.1 = false;

        nebula_log::info!("Started watching {} sources", sources.len());

        Ok(())
    }

    async fn stop_watching(&self) -> ConfigResult<()> {
        // Check if actually watching
        if !self.watching.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Stop the watcher
        {
            let mut watcher = self.watcher.write().await;
            *watcher = None;
        }

        // Clear the event sender
        {
            let mut event_tx = self.event_tx.write().await;
            *event_tx = None;
        }

        // Clear path mapping
        {
            let mut mapping = self.path_to_source.write().await;
            mapping.clear();
        }

        // Mark as not watching
        self.watching.store(false, Ordering::Relaxed);

        nebula_log::info!("Stopped watching");

        Ok(())
    }

    fn is_watching(&self) -> bool {
        self.watching.load(Ordering::Relaxed)
    }
}
