//! Simple and fast logger implementation

use std::path::PathBuf;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

// Type alias for better readability and to satisfy clippy
type BoxError = Box<dyn core::error::Error>;

/// Output format options
#[derive(Debug, Clone)]
pub enum Format {
    /// Pretty multi-line format with colors
    Pretty,
    /// Compact single-line format
    Compact,
    /// JSON format for machine processing
    Json,
}

/// Simple logger builder
#[derive(Debug, Clone)]
pub struct Logger {
    level: String,
    format: Format,
    colors: bool,
    source: bool,
    target: bool,
    time: bool,
    file: Option<PathBuf>,
}

impl Default for Logger {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: Format::Pretty,
            colors: cfg!(feature = "colors"),
            source: false,
            target: true,
            time: true,
            file: None,
        }
    }
}

impl Logger {
    /// Create a new logger builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set log level (trace, debug, info, warn, error)
    pub fn level<S: AsRef<str>>(mut self, level: S) -> Self {
        self.level = level.as_ref().to_string();
        self
    }

    /// Set output format
    pub fn format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// Enable/disable colors
    pub fn with_colors(mut self, colors: bool) -> Self {
        self.colors = colors;
        self
    }

    /// Show source file and line number
    pub fn with_source(mut self, source: bool) -> Self {
        self.source = source;
        self
    }

    /// Show target module
    pub fn with_target(mut self, target: bool) -> Self {
        self.target = target;
        self
    }

    /// Show timestamp
    pub fn with_time(mut self, time: bool) -> Self {
        self.time = time;
        self
    }

    /// Log to file
    pub fn to_file<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.file = Some(path.into());
        self
    }

    /// Initialize the logger
    pub fn init(self) -> Result<(), BoxError> {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(&self.level));

        match self.format {
            Format::Json => {
                let fmt_layer = fmt::layer()
                    .json()
                    .with_ansi(false);

                tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .init();
            }
            Format::Pretty => {
                let fmt_layer = fmt::layer()
                    .pretty()
                    .with_ansi(self.colors)
                    .with_target(self.target)
                    .with_file(self.source)
                    .with_line_number(self.source)
                    .with_timer(if self.time {
                        fmt::time::SystemTime
                    } else {
                        fmt::time()
                    });

                tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .init();
            }
            Format::Compact => {
                let fmt_layer = fmt::layer()
                    .compact()
                    .with_ansi(self.colors)
                    .with_target(self.target)
                    .with_timer(if self.time {
                        fmt::time::SystemTime
                    } else {
                        fmt::time()
                    });

                tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .init();
            }
        }

        Ok(())
    }

    /// Quick initialization with defaults
    pub fn init_default() -> Result<(), BoxError> {
        Self::new().init()
    }

    /// Initialize for development (pretty, debug level, colors, source)
    pub fn init_dev() -> Result<(), BoxError> {
        Self::new()
            .level("debug")
            .format(Format::Pretty)
            .with_colors(true)
            .with_source(true)
            .init()
    }

    /// Initialize for production (JSON, info level, no colors)
    pub fn init_production() -> Result<(), BoxError> {
        Self::new()
            .level("info")
            .format(Format::Json)
            .with_colors(false)
            .with_source(false)
            .init()
    }

    /// Initialize minimal logger for CLI tools
    pub fn init_minimal() -> Result<(), BoxError> {
        Self::new()
            .level("warn")
            .format(Format::Compact)
            .with_colors(false)
            .with_target(false)
            .with_time(false)
            .init()
    }

    /// Initialize with compact format
    pub fn init_compact() -> Result<(), BoxError> {
        Self::new()
            .format(Format::Compact)
            .init()
    }

    /// Initialize with JSON format
    pub fn init_json() -> Result<(), BoxError> {
        Self::new()
            .format(Format::Json)
            .init()
    }
}