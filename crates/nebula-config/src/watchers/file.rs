//! File system watcher for configuration files

use crate::core::{ConfigError, ConfigResult, ConfigSource};
use crate::watchers::{ConfigWatchEvent, ConfigWatchEventType};
use async_trait::async_trait;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{RwLock, mpsc};

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
        }
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

    /// Convert notify event to config watch event
    ///
    /// TODO: Use this helper in file watching implementation to convert
    /// raw notify events to ConfigWatchEvent with proper categorization.
    #[allow(dead_code)]
    fn convert_event(&self, event: Event, source: &ConfigSource) -> Option<ConfigWatchEvent> {
        let event_type = match event.kind {
            EventKind::Create(_) => ConfigWatchEventType::Created,
            EventKind::Modify(_) => ConfigWatchEventType::Modified,
            EventKind::Remove(_) => ConfigWatchEventType::Deleted,
            EventKind::Other => {
                return None; // Skip unknown events
            }
            _ => ConfigWatchEventType::Other("Unknown".to_string()),
        };

        let path = event.paths.first().cloned();

        Some(ConfigWatchEvent::new(event_type, source.clone()).with_path(path.unwrap_or_default()))
    }

    /// Start the event processing task
    async fn start_event_processor(&self, mut rx: mpsc::Receiver<ConfigWatchEvent>) {
        let callback = Arc::clone(&self.callback);
        let debounce_duration = self.debounce_duration;

        tokio::spawn(async move {
            let mut last_events: HashMap<PathBuf, std::time::Instant> = HashMap::new();

            while let Some(event) = rx.recv().await {
                // Debounce events for the same path
                if let Some(path) = &event.path {
                    let now = std::time::Instant::now();

                    if let Some(last_time) = last_events.get(path) {
                        if now.duration_since(*last_time) < debounce_duration {
                            continue; // Skip this event
                        }
                    }

                    last_events.insert(path.clone(), now);
                }

                // Call the callback
                (callback)(event);
            }
        });
    }
}

#[async_trait]
impl crate::core::ConfigWatcher for FileWatcher {
    async fn start_watching(&self, sources: &[ConfigSource]) -> ConfigResult<()> {
        // Check if already watching
        if self.watching.load(Ordering::Relaxed) {
            return Err(ConfigError::watch_error("Already watching"));
        }

        // Create channel for events
        let (tx, rx) = mpsc::channel(100);

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
                }
                ConfigSource::Directory(dir) => {
                    path_to_source.insert(dir.clone(), source.clone());
                    paths_to_watch.push(dir.clone());
                }
                _ => {
                    // Skip non-file sources
                }
            }
        }

        if paths_to_watch.is_empty() {
            nebula_log::debug!("No file sources to watch");
            return Ok(());
        }

        // Store the mapping
        {
            let mut mapping = self.path_to_source.write().await;
            *mapping = path_to_source.clone();
        }

        // Create file system watcher
        let tx_clone = tx.clone();
        let path_mapping = Arc::clone(&self.path_to_source);

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

                                if let Err(e) = tx_clone.blocking_send(watch_event) {
                                    nebula_log::error!("Failed to send watch event: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        nebula_log::error!("Watch error: {}", e);

                        let error_event = ConfigWatchEvent::new(
                            ConfigWatchEventType::Error(e.to_string()),
                            ConfigSource::File(PathBuf::new()),
                        );

                        let _ = tx_clone.blocking_send(error_event);
                    }
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

        // Start event processor
        self.start_event_processor(rx).await;

        // Mark as watching
        self.watching.store(true, Ordering::Relaxed);

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
