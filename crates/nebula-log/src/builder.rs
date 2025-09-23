//! Logger builder implementation

// Standard library
use std::sync::Arc;

// External dependencies
use parking_lot::Mutex;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

// Internal crates
use crate::{config::*, layer, writer, core::{LogResult}};

/// Logger builder
pub struct LoggerBuilder {
    config: Config,
}

/// Guard that keeps the logger alive
pub struct LoggerGuard {
    #[allow(dead_code)]
    inner: Option<Arc<Inner>>,
}

struct Inner {
    #[cfg(feature = "file")]
    file_guards: Vec<tracing_appender::non_blocking::WorkerGuard>,
    #[cfg(feature = "sentry")]
    sentry_guard: Option<sentry::ClientInitGuard>,
    reload_handle: Option<ReloadHandle>,
    _root_span_guard: Option<tracing::span::EnteredSpan>,
}

/// Handle for runtime configuration changes
#[derive(Clone)]
pub struct ReloadHandle {
    #[allow(dead_code)]
    filter: tracing_subscriber::reload::Handle<EnvFilter, Registry>,
    #[allow(dead_code)]
    current_filter: Arc<Mutex<String>>,
}

impl LoggerBuilder {
    /// Create builder from config
    pub fn from_config(config: Config) -> Self {
        Self { config }
    }

    /// Build and initialize the logger
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
        let filter =
            EnvFilter::try_new(&self.config.level).map_err(|e| {
                use crate::core::LogError;
                nebula_error::NebulaError::log_filter_error(&self.config.level, e.to_string())
            })?;

        // Get writer for the format layer
        let (writer, _guards) = writer::make_writer(&self.config.writer)?;

        #[cfg(feature = "file")]
        {
            inner.file_guards.extend(_guards);
        }

        // Build the subscriber based on reloadable flag and format
        match (self.config.reloadable, self.config.format) {
            (true, Format::Pretty) => self.build_reloadable_pretty(filter, writer, &mut inner)?,
            (true, Format::Compact) => self.build_reloadable_compact(filter, writer, &mut inner)?,
            (true, Format::Json) => self.build_reloadable_json(filter, writer, &mut inner)?,
            (true, Format::Logfmt) => self.build_reloadable_compact(filter, writer, &mut inner)?,
            (false, Format::Pretty) => self.build_static_pretty(filter, writer, &mut inner)?,
            (false, Format::Compact) => self.build_static_compact(filter, writer, &mut inner)?,
            (false, Format::Json) => self.build_static_json(filter, writer, &mut inner)?,
            (false, Format::Logfmt) => self.build_static_compact(filter, writer, &mut inner)?,
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
            inner: Some(Arc::new(inner)),
        })
    }

    // Reloadable variants

    fn build_reloadable_pretty(
        &self,
        filter: EnvFilter,
        writer: tracing_subscriber::fmt::writer::BoxMakeWriter,
        inner: &mut Inner,
    ) -> LogResult<()> {
        // Create a reloadable filter layer
        let (layer, handle) = tracing_subscriber::reload::Layer::new(filter);
        let reload = ReloadHandle {
            filter: handle,
            current_filter: Arc::new(Mutex::new(self.config.level.clone())),
        };
        inner.reload_handle = Some(reload.clone());

        // Build the subscriber
        let subscriber = Registry::default()
            .with(layer)
            .with(
                fmt::layer()
                    .pretty()
                    .with_writer(writer)
                    .with_ansi(self.config.display.colors)
                    .with_target(self.config.display.target)
                    .with_file(self.config.display.source)
                    .with_line_number(self.config.display.source)
                    .with_thread_ids(self.config.display.thread_ids)
                    .with_thread_names(self.config.display.thread_names),
            )
            .with(layer::fields::FieldsLayer::new(self.config.fields.clone()));

        #[cfg(feature = "sentry")]
        if let Some(guard) = crate::telemetry::sentry::init() {
            inner.sentry_guard = Some(guard);
        }

        // Initialize the subscriber (attach Sentry layer if enabled)
        #[cfg(feature = "sentry")]
        {
            let subscriber = subscriber.with(sentry_tracing::layer().event_filter(|md| {
                use sentry_tracing::EventFilter;
                match *md.level() {
                    tracing::Level::ERROR => EventFilter::Event,
                    tracing::Level::WARN => EventFilter::Breadcrumb,
                    _ => EventFilter::Ignore,
                }
            }));
            subscriber.init();
        }
        #[cfg(not(feature = "sentry"))]
        {
            subscriber.init();
        }

        // Bridge log crate if enabled
        #[cfg(feature = "log-compat")]
        {
            let _ = tracing_log::LogTracer::init();
        }

        Ok(())
    }

    fn build_reloadable_compact(
        &self,
        filter: EnvFilter,
        writer: tracing_subscriber::fmt::writer::BoxMakeWriter,
        inner: &mut Inner,
    ) -> LogResult<()> {
        // Create a reloadable filter layer
        let (layer, handle) = tracing_subscriber::reload::Layer::new(filter);
        let reload = ReloadHandle {
            filter: handle,
            current_filter: Arc::new(Mutex::new(self.config.level.clone())),
        };
        inner.reload_handle = Some(reload.clone());

        // Build the format layer
        let mut fmt_layer = fmt::layer()
            .compact()
            .with_writer(writer)
            .with_ansi(self.config.display.colors)
            .with_target(self.config.display.target)
            .with_file(self.config.display.source)
            .with_line_number(self.config.display.source)
            .with_thread_ids(self.config.display.thread_ids)
            .with_thread_names(self.config.display.thread_names);

        fmt_layer = fmt_layer.with_timer(crate::format::make_timer(if self.config.display.time {
            self.config.display.time_format.as_deref()
        } else {
            None
        }));

        let subscriber = Registry::default()
            .with(layer)
            .with(fmt_layer)
            .with(layer::fields::FieldsLayer::new(self.config.fields.clone()));

        #[cfg(feature = "sentry")]
        if let Some(guard) = crate::telemetry::sentry::init() {
            inner.sentry_guard = Some(guard);
        }

        // Initialize the subscriber (attach Sentry layer if enabled)
        #[cfg(feature = "sentry")]
        {
            let subscriber = subscriber.with(sentry_tracing::layer().event_filter(|md| {
                use sentry_tracing::EventFilter;
                match *md.level() {
                    tracing::Level::ERROR => EventFilter::Event,
                    tracing::Level::WARN => EventFilter::Breadcrumb,
                    _ => EventFilter::Ignore,
                }
            }));
            subscriber.init();
        }
        #[cfg(not(feature = "sentry"))]
        {
            subscriber.init();
        }

        // Bridge log crate if enabled
        #[cfg(feature = "log-compat")]
        {
            let _ = tracing_log::LogTracer::init();
        }

        Ok(())
    }

    fn build_reloadable_json(
        &self,
        filter: EnvFilter,
        writer: tracing_subscriber::fmt::writer::BoxMakeWriter,
        inner: &mut Inner,
    ) -> LogResult<()> {
        // Create a reloadable filter layer
        let (layer, handle) = tracing_subscriber::reload::Layer::new(filter);
        let reload = ReloadHandle {
            filter: handle,
            current_filter: Arc::new(Mutex::new(self.config.level.clone())),
        };
        inner.reload_handle = Some(reload.clone());

        // Build the format layer
        let mut fmt_layer = fmt::layer()
            .json()
            .with_writer(writer)
            .with_current_span(true)
            .with_span_list(self.config.display.span_list)
            .flatten_event(self.config.display.flatten)
            .with_ansi(self.config.display.colors)
            .with_target(self.config.display.target)
            .with_file(self.config.display.source)
            .with_line_number(self.config.display.source)
            .with_thread_ids(self.config.display.thread_ids)
            .with_thread_names(self.config.display.thread_names);

        fmt_layer = fmt_layer.with_timer(crate::format::make_timer(if self.config.display.time {
            self.config.display.time_format.as_deref()
        } else {
            None
        }));

        let subscriber = Registry::default()
            .with(layer)
            .with(fmt_layer)
            .with(layer::fields::FieldsLayer::new(self.config.fields.clone()));

        #[cfg(feature = "sentry")]
        if let Some(guard) = crate::telemetry::sentry::init() {
            inner.sentry_guard = Some(guard);
        }

        // Initialize the subscriber (attach Sentry layer if enabled)
        #[cfg(feature = "sentry")]
        {
            let subscriber = subscriber.with(sentry_tracing::layer().event_filter(|md| {
                use sentry_tracing::EventFilter;
                match *md.level() {
                    tracing::Level::ERROR => EventFilter::Event,
                    tracing::Level::WARN => EventFilter::Breadcrumb,
                    _ => EventFilter::Ignore,
                }
            }));
            subscriber.init();
        }
        #[cfg(not(feature = "sentry"))]
        {
            subscriber.init();
        }

        // Bridge log crate if enabled
        #[cfg(feature = "log-compat")]
        {
            let _ = tracing_log::LogTracer::init();
        }

        Ok(())
    }

    // Static (non-reloadable) variants

    fn build_static_pretty(
        &self,
        filter: EnvFilter,
        writer: tracing_subscriber::fmt::writer::BoxMakeWriter,
        _inner: &mut Inner,
    ) -> LogResult<()> {
        // Build the format layer
        let mut fmt_layer = fmt::layer()
            .pretty()
            .with_writer(writer)
            .with_ansi(self.config.display.colors)
            .with_target(self.config.display.target)
            .with_file(self.config.display.source)
            .with_line_number(self.config.display.source)
            .with_thread_ids(self.config.display.thread_ids)
            .with_thread_names(self.config.display.thread_names);

        fmt_layer = fmt_layer.with_timer(crate::format::make_timer(if self.config.display.time {
            self.config.display.time_format.as_deref()
        } else {
            None
        }));

        let subscriber = Registry::default()
            .with(filter)
            .with(fmt_layer)
            .with(layer::fields::FieldsLayer::new(self.config.fields.clone()));

        #[cfg(feature = "sentry")]
        if let Some(guard) = crate::telemetry::sentry::init() {
            _inner.sentry_guard = Some(guard);
        }

        // Initialize the subscriber (attach Sentry layer if enabled)
        #[cfg(feature = "sentry")]
        {
            let subscriber = subscriber.with(sentry_tracing::layer().event_filter(|md| {
                use sentry_tracing::EventFilter;
                match *md.level() {
                    tracing::Level::ERROR => EventFilter::Event,
                    tracing::Level::WARN => EventFilter::Breadcrumb,
                    _ => EventFilter::Ignore,
                }
            }));
            subscriber.init();
        }
        #[cfg(not(feature = "sentry"))]
        {
            subscriber.init();
        }

        // Bridge log crate if enabled
        #[cfg(feature = "log-compat")]
        {
            let _ = tracing_log::LogTracer::init();
        }

        Ok(())
    }

    fn build_static_compact(
        &self,
        filter: EnvFilter,
        writer: tracing_subscriber::fmt::writer::BoxMakeWriter,
        _inner: &mut Inner,
    ) -> LogResult<()> {
        // Build the format layer
        let mut fmt_layer = fmt::layer()
            .compact()
            .with_writer(writer)
            .with_ansi(self.config.display.colors)
            .with_target(self.config.display.target)
            .with_file(self.config.display.source)
            .with_line_number(self.config.display.source)
            .with_thread_ids(self.config.display.thread_ids)
            .with_thread_names(self.config.display.thread_names);

        fmt_layer = fmt_layer.with_timer(crate::format::make_timer(if self.config.display.time {
            self.config.display.time_format.as_deref()
        } else {
            None
        }));

        let subscriber = Registry::default()
            .with(filter)
            .with(fmt_layer)
            .with(layer::fields::FieldsLayer::new(self.config.fields.clone()));

        #[cfg(feature = "sentry")]
        if let Some(guard) = crate::telemetry::sentry::init() {
            _inner.sentry_guard = Some(guard);
        }

        // Initialize the subscriber (attach Sentry layer if enabled)
        #[cfg(feature = "sentry")]
        {
            let subscriber = subscriber.with(sentry_tracing::layer().event_filter(|md| {
                use sentry_tracing::EventFilter;
                match *md.level() {
                    tracing::Level::ERROR => EventFilter::Event,
                    tracing::Level::WARN => EventFilter::Breadcrumb,
                    _ => EventFilter::Ignore,
                }
            }));
            subscriber.init();
        }
        #[cfg(not(feature = "sentry"))]
        {
            subscriber.init();
        }

        // Bridge log crate if enabled
        #[cfg(feature = "log-compat")]
        {
            let _ = tracing_log::LogTracer::init();
        }

        Ok(())
    }

    fn build_static_json(
        &self,
        filter: EnvFilter,
        writer: tracing_subscriber::fmt::writer::BoxMakeWriter,
        _inner: &mut Inner,
    ) -> LogResult<()> {
        // Build the format layer
        let mut fmt_layer = fmt::layer()
            .json()
            .with_writer(writer)
            .with_current_span(true)
            .with_span_list(self.config.display.span_list)
            .flatten_event(self.config.display.flatten)
            .with_ansi(self.config.display.colors)
            .with_target(self.config.display.target)
            .with_file(self.config.display.source)
            .with_line_number(self.config.display.source)
            .with_thread_ids(self.config.display.thread_ids)
            .with_thread_names(self.config.display.thread_names);

        fmt_layer = fmt_layer.with_timer(crate::format::make_timer(if self.config.display.time {
            self.config.display.time_format.as_deref()
        } else {
            None
        }));

        let subscriber = Registry::default()
            .with(filter)
            .with(fmt_layer)
            .with(layer::fields::FieldsLayer::new(self.config.fields.clone()));

        #[cfg(feature = "sentry")]
        if let Some(guard) = crate::telemetry::sentry::init() {
            _inner.sentry_guard = Some(guard);
        }

        // Initialize the subscriber
        subscriber.init();

        // Bridge log crate if enabled
        #[cfg(feature = "log-compat")]
        {
            let _ = tracing_log::LogTracer::init();
        }

        Ok(())
    }
}

impl ReloadHandle {
    /// Reload the log filter
    #[allow(dead_code)]
    pub fn reload(&self, filter: &str) -> LogResult<()> {
        use crate::core::LogError;
        let new_filter = EnvFilter::try_new(filter).map_err(|e| nebula_error::NebulaError::log_filter_error(filter, e.to_string()))?;
        self.filter.reload(new_filter).map_err(|e| nebula_error::NebulaError::log_config_error(format!("Failed to reload filter: {}", e)))?;
        *self.current_filter.lock() = filter.to_string();
        Ok(())
    }

    /// Get current filter string
    #[allow(dead_code)]
    pub fn current_filter(&self) -> String {
        self.current_filter.lock().clone()
    }
}

impl LoggerGuard {
    #[cfg(test)]
    pub(crate) fn noop() -> Self {
        Self { inner: None }
    }
}
