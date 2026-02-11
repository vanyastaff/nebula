//! Logger builder implementation
//!
//! This module is organized into:
//! - `format`: Format layer creation macros (reduces duplication)
//! - `reload`: Runtime filter reload logic
//! - `telemetry`: Sentry and log bridge integration

#[macro_use]
mod format;
#[macro_use]
mod telemetry;
mod reload;

// Re-export public types
pub use reload::ReloadHandle;

// External dependencies
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt, util::SubscriberInitExt};

// Internal crates
use crate::core::LogResult;
use crate::{
    config::{Config, Format},
    writer,
};

/// Logger builder
#[derive(Debug)]
pub struct LoggerBuilder {
    config: Config,
}

/// Guard that keeps the logger alive
///
/// This guard ensures that all logging infrastructure stays alive for the lifetime
/// of the guard. When dropped, the logger will be properly shut down.
#[derive(Debug)]
pub struct LoggerGuard {
    /// RAII guard - field must exist even if never accessed directly
    /// to keep file guards and other resources alive
    #[allow(dead_code)]
    inner: Option<Box<Inner>>,
}

pub(crate) struct Inner {
    #[cfg(feature = "file")]
    pub(crate) file_guards: Vec<tracing_appender::non_blocking::WorkerGuard>,
    #[cfg(feature = "sentry")]
    pub(crate) sentry_guard: Option<sentry::ClientInitGuard>,
    pub(crate) reload_handle: Option<ReloadHandle>,
    /// RAII guard for root span - intentionally prefixed with _ to indicate it's never accessed
    #[allow(clippy::used_underscore_binding)]
    pub(crate) _root_span_guard: Option<tracing::span::EnteredSpan>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner").finish_non_exhaustive()
    }
}

/// Helper macro to build and init the subscriber for a given format layer.
/// Avoids repeating the same `Registry + filter + fmt + fields + sentry` chain.
/// The FieldsLayer is only added when fields are non-empty (avoids a no-op layer).
macro_rules! init_subscriber {
    ($filter_layer:expr, $fmt_layer:expr, $fields:expr) => {{
        let fields = $fields.clone();
        if fields.is_empty() {
            let subscriber = Registry::default().with($filter_layer).with($fmt_layer);
            attach_sentry!(subscriber).init();
        } else {
            let subscriber = Registry::default()
                .with($filter_layer)
                .with($fmt_layer)
                .with(crate::layer::fields::FieldsLayer::new(fields));
            attach_sentry!(subscriber).init();
        }
    }};
}

impl LoggerBuilder {
    /// Create builder from config
    #[must_use]
    pub fn from_config(config: Config) -> Self {
        Self { config }
    }

    /// Build and initialize the logger
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Filter string cannot be parsed
    /// - File writer initialization fails
    /// - Telemetry setup fails
    pub fn build(self) -> LogResult<LoggerGuard> {
        let mut inner = Inner {
            #[cfg(feature = "file")]
            file_guards: Vec::new(),
            #[cfg(feature = "sentry")]
            sentry_guard: None,
            reload_handle: None,
            _root_span_guard: None,
        };

        // Create the filter
        let filter = EnvFilter::try_new(&self.config.level).map_err(|e| {
            use crate::core::LogError;
            LogError::Filter(format!("{}: {}", &self.config.level, e))
        })?;

        // Get writer for the format layer
        let (writer, _guards) = writer::make_writer(&self.config.writer)?;

        #[cfg(feature = "file")]
        {
            inner.file_guards.extend(_guards);
        }

        // Create filter layer (optionally reloadable)
        let (filter_layer, reload_handle) =
            reload::create_filter_layer(filter, &self.config.level, self.config.reloadable);

        inner.reload_handle = reload_handle;

        // Initialize telemetry (Sentry + log bridge)
        telemetry::init_telemetry(&mut inner);

        // Build subscriber based on format
        match self.config.format {
            Format::Pretty => {
                let fmt_layer = create_fmt_layer!(pretty, &self.config.display, writer);
                init_subscriber!(filter_layer, fmt_layer, self.config.fields);
            }
            Format::Compact | Format::Logfmt => {
                let fmt_layer = create_fmt_layer!(compact, &self.config.display, writer);
                init_subscriber!(filter_layer, fmt_layer, self.config.fields);
            }
            Format::Json => {
                let fmt_layer = create_json_layer!(&self.config.display, writer);
                init_subscriber!(filter_layer, fmt_layer, self.config.fields);
            }
        }

        // Create root span with global fields
        if !self.config.fields.is_empty() {
            let root = tracing::info_span!(
                "app",
                service = self.config.fields.service.as_deref().unwrap_or(""),
                env = self.config.fields.env.as_deref().unwrap_or(""),
                version = self.config.fields.version.as_deref().unwrap_or(""),
                instance = self.config.fields.instance.as_deref().unwrap_or(""),
                region = self.config.fields.region.as_deref().unwrap_or("")
            );
            inner._root_span_guard = Some(root.entered());
        }

        Ok(LoggerGuard {
            inner: Some(Box::new(inner)),
        })
    }
}

impl LoggerGuard {
    #[cfg(test)]
    pub(crate) fn noop() -> Self {
        Self { inner: None }
    }
}
