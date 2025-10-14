//! Reload logic for runtime filter changes

use parking_lot::Mutex;
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, Registry, layer::Layer};

use crate::core::LogResult;

/// Handle for runtime configuration changes
#[derive(Clone)]
pub struct ReloadHandle {
    /// Filter reload handle - used by public reload() method
    #[allow(dead_code)]
    filter: tracing_subscriber::reload::Handle<EnvFilter, Registry>,
    /// Current filter string - used by public current_filter() method
    #[allow(dead_code)]
    current_filter: Arc<Mutex<String>>,
}

impl ReloadHandle {
    /// Reload the log filter at runtime
    ///
    /// # Errors
    /// Returns error if filter parsing fails or reload fails
    #[allow(dead_code)]
    pub fn reload(&self, filter: &str) -> LogResult<()> {
        use crate::core::LogError;
        let new_filter = EnvFilter::try_new(filter)
            .map_err(|e| LogError::filter_parsing_error(format!("{}: {}", filter, e)))?;
        self.filter
            .reload(new_filter)
            .map_err(|e| LogError::configuration_error(format!("Failed to reload filter: {e}")))?;
        *self.current_filter.lock() = filter.to_string();
        Ok(())
    }

    /// Get the current filter string
    #[allow(dead_code)]
    pub fn current_filter(&self) -> String {
        self.current_filter.lock().clone()
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
            current_filter: Arc::new(Mutex::new(level_str.to_string())),
        };
        (Box::new(layer), Some(reload_handle))
    } else {
        (Box::new(filter), None)
    }
}
