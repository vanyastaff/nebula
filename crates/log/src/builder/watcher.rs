//! Config file watcher for automatic log-level reloading.
//!
//! Polls a config file at a fixed interval and, when the content changes,
//! reloads the log filter via [`ReloadHandle`].

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::sync::watch;

use super::ReloadHandle;

/// Default poll interval for the config watcher.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Guard that stops the config watcher when dropped.
///
/// Created by [`watch_config`] or [`watch_config_with_interval`].
#[derive(Debug)]
pub struct WatcherGuard {
    _cancel: watch::Sender<()>,
}

/// Start watching a config file for log-level changes.
///
/// The file should contain a plain-text filter string (e.g. `"info,nebula_engine=debug"`).
/// Leading/trailing whitespace is trimmed. Empty or whitespace-only content is ignored.
///
/// The watcher spawns a background tokio task that polls the file every 5 seconds.
/// Drop the returned [`WatcherGuard`] to stop watching.
///
/// # Examples
///
/// ```ignore
/// let guard = nebula_log::init_with(Config { reloadable: true, ..Config::default() })?;
/// let handle = guard.reload_handle().expect("reloadable was true");
/// let watcher = nebula_log::builder::watcher::watch_config("log-level.conf", handle.clone());
/// // ... watcher auto-reloads when log-level.conf changes
/// drop(watcher); // stops polling
/// ```
pub fn watch_config(path: impl Into<PathBuf>, handle: ReloadHandle) -> WatcherGuard {
    watch_config_with_interval(path, handle, DEFAULT_POLL_INTERVAL)
}

/// Like [`watch_config`] but with a custom poll interval.
pub fn watch_config_with_interval(
    path: impl Into<PathBuf>,
    handle: ReloadHandle,
    interval: Duration,
) -> WatcherGuard {
    let path = path.into();
    let (cancel_tx, cancel_rx) = watch::channel(());

    tokio::spawn(watcher_task(path, handle, interval, cancel_rx));

    WatcherGuard { _cancel: cancel_tx }
}

async fn watcher_task(
    path: PathBuf,
    handle: ReloadHandle,
    interval: Duration,
    mut cancel: watch::Receiver<()>,
) {
    tracing::info!(path = %path.display(), "watching config file for log level changes");

    let mut last_content = read_filter(&path).await;

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = cancel.changed() => {
                tracing::debug!(path = %path.display(), "config watcher stopped");
                return;
            }
        }

        let content = read_filter(&path).await;

        // Skip if unchanged or empty
        if content == last_content || content.is_none() {
            continue;
        }

        if let Some(ref new_filter) = content {
            match handle.reload(new_filter) {
                Ok(()) => {
                    tracing::info!(
                        path = %path.display(),
                        filter = %new_filter,
                        "detected config change, reloaded log filter"
                    );
                },
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to reload log filter from config file"
                    );
                },
            }
        }

        last_content = content;
    }
}

/// Read the file and return the trimmed content, or `None` on error/empty.
async fn read_filter(path: &Path) -> Option<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        },
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to read config file for log level reload"
            );
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[tokio::test]
    async fn watcher_detects_file_change() {
        // Create a reloadable filter
        let filter = tracing_subscriber::EnvFilter::try_new("info").unwrap();
        let (_layer, handle) = super::super::reload::create_filter_layer(filter, "info", true);
        let handle = handle.unwrap();

        // Write initial content
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "info").unwrap();
        file.flush().unwrap();

        // Start watcher with short interval
        let _guard = watch_config_with_interval(
            file.path().to_path_buf(),
            handle.clone(),
            Duration::from_millis(50),
        );

        // Let the watcher read the initial value
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Update file
        std::fs::write(file.path(), "debug,nebula_engine=trace").unwrap();

        // Wait for watcher to pick up the change
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(*handle.current_filter(), "debug,nebula_engine=trace");
    }

    #[tokio::test]
    async fn watcher_ignores_invalid_filter() {
        let filter = tracing_subscriber::EnvFilter::try_new("info").unwrap();
        let (_layer, handle) = super::super::reload::create_filter_layer(filter, "info", true);
        let handle = handle.unwrap();

        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "info").unwrap();
        file.flush().unwrap();

        let _guard = watch_config_with_interval(
            file.path().to_path_buf(),
            handle.clone(),
            Duration::from_millis(50),
        );

        // Write invalid filter
        std::fs::write(file.path(), "=====invalid=====").unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Should still be the original filter
        assert_eq!(*handle.current_filter(), "info");
    }

    #[tokio::test]
    async fn watcher_stops_on_guard_drop() {
        let filter = tracing_subscriber::EnvFilter::try_new("info").unwrap();
        let (_layer, handle) = super::super::reload::create_filter_layer(filter, "info", true);
        let handle = handle.unwrap();

        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "info").unwrap();
        file.flush().unwrap();

        let guard = watch_config_with_interval(
            file.path().to_path_buf(),
            handle.clone(),
            Duration::from_millis(50),
        );

        // Drop the guard to stop watcher
        drop(guard);

        // Update file after guard is dropped
        std::fs::write(file.path(), "debug").unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Filter should remain unchanged since watcher was stopped
        assert_eq!(*handle.current_filter(), "info");
    }
}
