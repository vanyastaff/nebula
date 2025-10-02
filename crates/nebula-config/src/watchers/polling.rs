//! Polling-based watcher for environments without native file watching

use crate::core::{ConfigError, ConfigResult, ConfigSource, ConfigWatcher};
use crate::watchers::{ConfigWatchEvent, ConfigWatchEventType};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;

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
    hash: Option<String>,
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
    pub fn with_interval(mut self, interval: std::time::Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Get file metadata
    async fn get_file_metadata(path: &PathBuf) -> Option<FileMetadata> {
        match tokio::fs::metadata(path).await {
            Ok(metadata) => {
                Some(FileMetadata {
                    modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                    size: metadata.len(),
                    hash: None, // Could add content hashing for better change detection
                })
            }
            Err(_) => None,
        }
    }

    /// Check if file has changed
    fn has_changed(old: &FileMetadata, new: &FileMetadata) -> bool {
        old.modified != new.modified || old.size != new.size
    }

    /// Start the polling loop
    async fn start_polling_loop(
        &self,
        sources: Vec<ConfigSource>,
        callback: Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
        watching: Arc<AtomicBool>,
        metadata_cache: Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
        interval: std::time::Duration,
    ) {
        let mut interval_timer = tokio::time::interval(interval);
        interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Initial scan to populate cache
        for source in &sources {
            match source {
                ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                    if let Some(metadata) = Self::get_file_metadata(path).await {
                        let mut cache = metadata_cache.write().await;
                        cache.insert(path.clone(), metadata);
                    }
                }
                ConfigSource::Directory(dir) => {
                    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
                        let mut cache = metadata_cache.write().await;

                        while let Ok(Some(entry)) = entries.next_entry().await {
                            let path = entry.path();
                            if let Some(metadata) = Self::get_file_metadata(&path).await {
                                cache.insert(path, metadata);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        while watching.load(Ordering::Relaxed) {
            interval_timer.tick().await;

            for source in &sources {
                match source {
                    ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                        self.check_file_changes(path, source, &callback, &metadata_cache)
                            .await;
                    }
                    ConfigSource::Directory(dir) => {
                        self.check_directory_changes(dir, source, &callback, &metadata_cache)
                            .await;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Check for file changes
    async fn check_file_changes(
        &self,
        path: &PathBuf,
        source: &ConfigSource,
        callback: &Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
        cache: &Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
    ) {
        let current_metadata = Self::get_file_metadata(path).await;
        let mut cache = cache.write().await;

        match (cache.get(path), current_metadata) {
            (Some(old), Some(new)) if Self::has_changed(old, &new) => {
                // File modified
                let event = ConfigWatchEvent::new(ConfigWatchEventType::Modified, source.clone())
                    .with_path(path.clone());

                callback(event);
                cache.insert(path.clone(), new);
            }
            (Some(_), None) => {
                // File deleted
                let event = ConfigWatchEvent::new(ConfigWatchEventType::Deleted, source.clone())
                    .with_path(path.clone());

                callback(event);
                cache.remove(path);
            }
            (None, Some(new)) => {
                // File created
                let event = ConfigWatchEvent::new(ConfigWatchEventType::Created, source.clone())
                    .with_path(path.clone());

                callback(event);
                cache.insert(path.clone(), new);
            }
            _ => {}
        }
    }

    /// Check for directory changes
    async fn check_directory_changes(
        &self,
        dir: &PathBuf,
        source: &ConfigSource,
        callback: &Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
        cache: &Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
    ) {
        let mut current_files = HashMap::new();

        // Scan directory
        if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if let Some(metadata) = Self::get_file_metadata(&path).await {
                    current_files.insert(path, metadata);
                }
            }
        }

        let mut cache = cache.write().await;

        // Check for changes
        for (path, new_metadata) in &current_files {
            if !path.starts_with(dir) {
                continue;
            }

            match cache.get(path) {
                Some(old) if Self::has_changed(old, new_metadata) => {
                    // File modified
                    let event =
                        ConfigWatchEvent::new(ConfigWatchEventType::Modified, source.clone())
                            .with_path(path.clone());

                    callback(event);
                }
                None => {
                    // File created
                    let event =
                        ConfigWatchEvent::new(ConfigWatchEventType::Created, source.clone())
                            .with_path(path.clone());

                    callback(event);
                }
                _ => {}
            }
        }

        // Check for deletions
        let paths_in_dir: Vec<_> = cache
            .keys()
            .filter(|p| p.starts_with(dir))
            .cloned()
            .collect();

        for path in paths_in_dir {
            if !current_files.contains_key(&path) {
                // File deleted
                let event = ConfigWatchEvent::new(ConfigWatchEventType::Deleted, source.clone())
                    .with_path(path.clone());

                callback(event);
                cache.remove(&path);
            }
        }

        // Update cache with current state
        for (path, metadata) in current_files {
            cache.insert(path, metadata);
        }
    }
}

#[async_trait]
impl ConfigWatcher for PollingWatcher {
    async fn start_watching(&self, sources: &[ConfigSource]) -> ConfigResult<()> {
        // Check if already watching
        if self.watching.load(Ordering::Relaxed) {
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

        // Mark as watching
        watching.store(true, Ordering::Relaxed);

        // Start polling task
        let watcher = self.clone();
        let handle = tokio::spawn(async move {
            watcher
                .start_polling_loop(sources, callback, watching, metadata_cache, interval)
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

        // Cancel the task
        {
            let mut task_handle = self.task_handle.write().await;
            if let Some(handle) = task_handle.take() {
                handle.abort();
                // Wait a bit for graceful shutdown
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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
