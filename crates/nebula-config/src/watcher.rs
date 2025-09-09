//! Configuration watching for hot-reload

use crate::{ConfigError, ConfigResult, ConfigSource};
use async_trait::async_trait;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Configuration watcher trait
#[async_trait]
pub trait ConfigWatcher: Send + Sync {
    /// Start watching configuration sources
    async fn start_watching(&self, sources: &[ConfigSource]) -> ConfigResult<()>;
    
    /// Stop watching
    async fn stop_watching(&self) -> ConfigResult<()>;
    
    /// Check if currently watching
    fn is_watching(&self) -> bool;
}

/// File system watcher for configuration files
pub struct FileWatcher {
    /// File system watcher
    watcher: Arc<tokio::sync::Mutex<Option<RecommendedWatcher>>>,
    
    /// Event sender
    event_tx: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedSender<ConfigWatchEvent>>>>,
    
    /// Event receiver
    event_rx: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<ConfigWatchEvent>>>>,
    
    /// Callback for configuration changes
    callback: Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
    
    /// Currently watching
    watching: Arc<tokio::sync::RwLock<bool>>,
}

impl std::fmt::Debug for FileWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWatcher")
            .field("watcher", &"<watcher>")
            .field("callback", &"<callback>")
            .field("watching", &"<watching>")
            .finish()
    }
}

/// Configuration watch event
#[derive(Debug, Clone)]
pub struct ConfigWatchEvent {
    /// Event type
    pub event_type: ConfigWatchEventType,
    
    /// Source that changed
    pub source: ConfigSource,
    
    /// File path (if applicable)
    pub path: Option<std::path::PathBuf>,
    
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Configuration watch event type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigWatchEventType {
    /// File created
    Created,
    
    /// File modified
    Modified,
    
    /// File deleted
    Deleted,
    
    /// File renamed
    Renamed,
    
    /// Other event
    Other,
}

impl FileWatcher {
    /// Create a new file watcher
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(ConfigWatchEvent) + Send + Sync + 'static,
    {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        Self {
            watcher: Arc::new(tokio::sync::Mutex::new(None)),
            event_tx: Arc::new(tokio::sync::Mutex::new(Some(event_tx))),
            event_rx: Arc::new(tokio::sync::Mutex::new(Some(event_rx))),
            callback: Arc::new(callback),
            watching: Arc::new(tokio::sync::RwLock::new(false)),
        }
    }
    
    /// Create a new file watcher with no-op callback
    pub fn new_noop() -> Self {
        Self::new(|_| {})
    }
    
    /// Convert notify event to config watch event
    fn convert_event(&self, event: Event, source: &ConfigSource) -> Option<ConfigWatchEvent> {
        let event_type = match event.kind {
            EventKind::Create(_) => ConfigWatchEventType::Created,
            EventKind::Modify(_) => ConfigWatchEventType::Modified,
            EventKind::Remove(_) => ConfigWatchEventType::Deleted,
            EventKind::Other => ConfigWatchEventType::Other,
            _ => return None,
        };
        
        let path = event.paths.first().cloned();
        
        Some(ConfigWatchEvent {
            event_type,
            source: source.clone(),
            path,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[async_trait]
impl ConfigWatcher for FileWatcher {
    async fn start_watching(&self, sources: &[ConfigSource]) -> ConfigResult<()> {
        let mut watcher_guard = self.watcher.lock().await;
        let mut event_tx_guard = self.event_tx.lock().await;
        
        if watcher_guard.is_some() {
            return Err(ConfigError::watch_error("Already watching"));
        }
        
        let event_tx = event_tx_guard.take().ok_or_else(|| {
            ConfigError::watch_error("Event sender already taken")
        })?;
        
        // Create file system watcher
        let mut fs_watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    // For simplicity, we'll send all events and filter later
                    // In a real implementation, you'd associate events with specific sources
                    if let Err(e) = event_tx.send(ConfigWatchEvent {
                        event_type: match event.kind {
                            EventKind::Create(_) => ConfigWatchEventType::Created,
                            EventKind::Modify(_) => ConfigWatchEventType::Modified,
                            EventKind::Remove(_) => ConfigWatchEventType::Deleted,
                            _ => ConfigWatchEventType::Other,
                        },
                        source: ConfigSource::File(event.paths.first().cloned().unwrap_or_default()),
                        path: event.paths.first().cloned(),
                        timestamp: chrono::Utc::now(),
                    }) {
                        nebula_log::error!("Failed to send watch event: {}", e);
                    }
                }
                Err(e) => {
                    nebula_log::error!("Watch error: {}", e);
                }
            }
        }).map_err(|e| ConfigError::watch_error(e.to_string()))?;
        
        // Watch file-based sources
        for source in sources {
            match source {
                ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                    if let Some(parent) = path.parent() {
                        fs_watcher.watch(parent, RecursiveMode::NonRecursive)
                            .map_err(|e| ConfigError::watch_error(e.to_string()))?;
                    } else {
                        fs_watcher.watch(path, RecursiveMode::NonRecursive)
                            .map_err(|e| ConfigError::watch_error(e.to_string()))?;
                    }
                }
                ConfigSource::Directory(path) => {
                    fs_watcher.watch(path, RecursiveMode::Recursive)
                        .map_err(|e| ConfigError::watch_error(e.to_string()))?;
                }
                _ => {
                    // Non-file sources are not watched by FileWatcher
                }
            }
        }
        
        *watcher_guard = Some(fs_watcher);
        
        // Start event processing task
        let callback = Arc::clone(&self.callback);
        let mut event_rx_guard = self.event_rx.lock().await;
        if let Some(mut event_rx) = event_rx_guard.take() {
            tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    callback(event);
                }
            });
        }
        
        // Mark as watching
        let mut watching = self.watching.write().await;
        *watching = true;
        
        Ok(())
    }
    
    async fn stop_watching(&self) -> ConfigResult<()> {
        let mut watcher_guard = self.watcher.lock().await;
        *watcher_guard = None;
        
        // Mark as not watching
        let mut watching = self.watching.write().await;
        *watching = false;
        
        Ok(())
    }
    
    fn is_watching(&self) -> bool {
        // This is a simplified check - in a real implementation,
        // you'd use a non-blocking read or store the state differently
        false
    }
}

/// No-op watcher that does nothing
#[derive(Debug, Clone)]
pub struct NoOpWatcher;

#[async_trait]
impl ConfigWatcher for NoOpWatcher {
    async fn start_watching(&self, _sources: &[ConfigSource]) -> ConfigResult<()> {
        Ok(())
    }
    
    async fn stop_watching(&self) -> ConfigResult<()> {
        Ok(())
    }
    
    fn is_watching(&self) -> bool {
        false
    }
}

/// Polling watcher that checks for changes at regular intervals
pub struct PollingWatcher {
    /// Polling interval
    interval: std::time::Duration,
    
    /// Callback for configuration changes
    callback: Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
    
    /// Currently watching
    watching: Arc<tokio::sync::RwLock<bool>>,
    
    /// Task handle
    task_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl std::fmt::Debug for PollingWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollingWatcher")
            .field("interval", &self.interval)
            .field("callback", &"<callback>")
            .field("watching", &"<watching>")
            .field("task_handle", &"<task_handle>")
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
            watching: Arc::new(tokio::sync::RwLock::new(false)),
            task_handle: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
    
    /// Create a new polling watcher with no-op callback
    pub fn new_noop(interval: std::time::Duration) -> Self {
        Self::new(interval, |_| {})
    }
}

#[async_trait]
impl ConfigWatcher for PollingWatcher {
    async fn start_watching(&self, sources: &[ConfigSource]) -> ConfigResult<()> {
        let mut task_handle_guard = self.task_handle.lock().await;
        
        if task_handle_guard.is_some() {
            return Err(ConfigError::watch_error("Already watching"));
        }
        
        // Store file metadata for comparison
        let mut file_metadata: std::collections::HashMap<std::path::PathBuf, std::fs::Metadata> = 
            std::collections::HashMap::new();
        
        // Initialize metadata for file sources
        for source in sources {
            match source {
                ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        file_metadata.insert(path.clone(), metadata);
                    }
                }
                _ => {}
            }
        }
        
        let sources = sources.to_vec();
        let interval = self.interval;
        let callback = Arc::clone(&self.callback);
        let watching = Arc::clone(&self.watching);
        
        let handle = tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            let mut last_metadata = file_metadata;
            
            loop {
                interval_timer.tick().await;
                
                // Check if still watching
                {
                    let watching_guard = watching.read().await;
                    if !*watching_guard {
                        break;
                    }
                }
                
                // Check each file source for changes
                for source in &sources {
                    match source {
                        ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                            if let Ok(current_metadata) = std::fs::metadata(path) {
                                let changed = if let Some(last_meta) = last_metadata.get(path) {
                                    current_metadata.modified().unwrap_or(std::time::UNIX_EPOCH) !=
                                    last_meta.modified().unwrap_or(std::time::UNIX_EPOCH)
                                } else {
                                    true // New file
                                };
                                
                                if changed {
                                    let event = ConfigWatchEvent {
                                        event_type: ConfigWatchEventType::Modified,
                                        source: source.clone(),
                                        path: Some(path.clone()),
                                        timestamp: chrono::Utc::now(),
                                    };
                                    
                                    callback(event);
                                    last_metadata.insert(path.clone(), current_metadata);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
        
        *task_handle_guard = Some(handle);
        
        // Mark as watching
        let mut watching = self.watching.write().await;
        *watching = true;
        
        Ok(())
    }
    
    async fn stop_watching(&self) -> ConfigResult<()> {
        // Mark as not watching
        {
            let mut watching = self.watching.write().await;
            *watching = false;
        }
        
        // Cancel the task
        let mut task_handle_guard = self.task_handle.lock().await;
        if let Some(handle) = task_handle_guard.take() {
            handle.abort();
        }
        
        Ok(())
    }
    
    fn is_watching(&self) -> bool {
        // This is a simplified check - in a real implementation,
        // you'd use a non-blocking read or store the state differently
        false
    }
}
