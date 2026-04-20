//! Reload logic for runtime filter changes

use std::sync::Arc;

use arc_swap::ArcSwap;
use tracing_subscriber::{EnvFilter, Registry, layer::Layer};

use crate::core::LogResult;

/// Handle for runtime configuration changes.
///
/// Obtained via [`super::LoggerGuard::reload_handle`] when the logger was built
/// with `reloadable: true`.
#[derive(Clone)]
pub struct ReloadHandle {
    filter: tracing_subscriber::reload::Handle<EnvFilter, Registry>,
    /// Current filter string — lock-free reads via ArcSwap
    current_filter: Arc<ArcSwap<String>>,
}

impl ReloadHandle {
    /// Reload the log filter at runtime.
    ///
    /// Accepts a [`tracing_subscriber::EnvFilter`]-compatible string
    /// (e.g. `"info,nebula_engine=debug"`).
    ///
    /// # Errors
    ///
    /// Returns error if filter parsing fails or the reload layer has been
    /// dropped.
    pub fn reload(&self, filter: &str) -> LogResult<()> {
        use crate::core::LogError;
        let new_filter =
            EnvFilter::try_new(filter).map_err(|e| LogError::Filter(format!("{filter}: {e}")))?;
        self.filter
            .reload(new_filter)
            .map_err(|e| LogError::Config(format!("Failed to reload filter: {e}")))?;
        self.current_filter.store(Arc::new(filter.to_string()));
        tracing::info!("log filter reloaded: {filter}");
        Ok(())
    }

    /// Get the current filter string.
    pub fn current_filter(&self) -> Arc<String> {
        self.current_filter.load_full()
    }
}

/// Create a filter layer, optionally wrapping it in a reloadable layer
///
/// Returns:
/// - The filter layer (potentially wrapped in reload)
/// - Optional reload handle (if reloadable=true)
pub(super) fn create_filter_layer(
    filter: EnvFilter,
    level_str: &str,
    reloadable: bool,
) -> (
    Box<dyn Layer<Registry> + Send + Sync + 'static>,
    Option<ReloadHandle>,
) {
    if reloadable {
        let (layer, handle) = tracing_subscriber::reload::Layer::new(filter);
        let reload_handle = ReloadHandle {
            filter: handle,
            current_filter: Arc::new(ArcSwap::from_pointee(level_str.to_string())),
        };
        (Box::new(layer), Some(reload_handle))
    } else {
        (Box::new(filter), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reloadable_true_returns_handle() {
        let filter = EnvFilter::try_new("info").unwrap();
        let (_layer, handle) = create_filter_layer(filter, "info", true);
        assert!(handle.is_some());
    }

    #[test]
    fn reloadable_false_returns_none() {
        let filter = EnvFilter::try_new("info").unwrap();
        let (_layer, handle) = create_filter_layer(filter, "info", false);
        assert!(handle.is_none());
    }

    #[test]
    fn current_filter_returns_initial_value() {
        let filter = EnvFilter::try_new("debug,hyper=warn").unwrap();
        let (_layer, handle) = create_filter_layer(filter, "debug,hyper=warn", true);
        let handle = handle.unwrap();
        assert_eq!(*handle.current_filter(), "debug,hyper=warn");
    }

    #[test]
    fn reload_updates_current_filter() {
        let filter = EnvFilter::try_new("info").unwrap();
        let (_layer, handle) = create_filter_layer(filter, "info", true);
        let handle = handle.unwrap();

        handle.reload("debug,nebula_engine=trace").unwrap();
        assert_eq!(*handle.current_filter(), "debug,nebula_engine=trace");
    }

    #[test]
    fn reload_rejects_invalid_filter() {
        let filter = EnvFilter::try_new("info").unwrap();
        let (_layer, handle) = create_filter_layer(filter, "info", true);
        let handle = handle.unwrap();

        // An empty filter string is valid for EnvFilter, so use something truly invalid
        let result = handle.reload("=====invalid=====");
        assert!(result.is_err());
        // Original filter should be unchanged
        assert_eq!(*handle.current_filter(), "info");
    }
}
