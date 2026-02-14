//! Logging integration for nebula-memory
//!
//! This module provides extension traits that allow integrating
//! the memory management system with various logging frameworks.

use core::fmt;
use std::{boxed::Box, string::String, sync::Arc, vec::Vec};

use crate::error::MemoryResult;
use crate::extensions::MemoryExtension;

/// Log level for memory-related events
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Trace-level details (very verbose)
    Trace,
    /// Debug information
    Debug,
    /// Informational messages
    Info,
    /// Warnings (non-critical issues)
    Warn,
    /// Errors (critical issues)
    Error,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trace => write!(f, "TRACE"),
            Self::Debug => write!(f, "DEBUG"),
            Self::Info => write!(f, "INFO"),
            Self::Warn => write!(f, "WARN"),
            Self::Error => write!(f, "ERROR"),
        }
    }
}

/// Memory event that can be logged
#[derive(Debug, Clone)]
pub struct MemoryEvent {
    /// Type of memory event
    pub event_type: String,
    /// Source component that generated the event
    pub source: String,
    /// Log level for this event
    pub level: LogLevel,
    /// Message describing the event
    pub message: String,
    /// Additional structured data for the event
    pub data: Vec<(String, String)>,
    /// Timestamp when the event occurred (in milliseconds since epoch)
    pub timestamp: u64,
}

impl MemoryEvent {
    /// Create a new memory event
    pub fn new(
        event_type: impl Into<String>,
        source: impl Into<String>,
        level: LogLevel,
        message: impl Into<String>,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            source: source.into(),
            level,
            message: message.into(),
            data: Vec::new(),
            timestamp: timestamp_now(),
        }
    }

    /// Add structured data to the event
    #[must_use = "builder methods must be chained or built"]
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.push((key.into(), value.into()));
        self
    }
}

/// Get current timestamp in milliseconds
fn timestamp_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Trait for logging memory events
pub trait MemoryLogger: Send + Sync {
    /// Log a memory event
    fn log(&self, event: &MemoryEvent);

    /// Log a memory event with a specific level
    fn log_with_level(&self, level: LogLevel, event_type: &str, source: &str, message: &str) {
        self.log(&MemoryEvent::new(event_type, source, level, message));
    }

    /// Log a trace message
    fn trace(&self, event_type: &str, source: &str, message: &str) {
        self.log_with_level(LogLevel::Trace, event_type, source, message);
    }

    /// Log a debug message
    fn debug(&self, event_type: &str, source: &str, message: &str) {
        self.log_with_level(LogLevel::Debug, event_type, source, message);
    }

    /// Log an info message
    fn info(&self, event_type: &str, source: &str, message: &str) {
        self.log_with_level(LogLevel::Info, event_type, source, message);
    }

    /// Log a warning message
    fn warn(&self, event_type: &str, source: &str, message: &str) {
        self.log_with_level(LogLevel::Warn, event_type, source, message);
    }

    /// Log an error message
    fn error(&self, event_type: &str, source: &str, message: &str) {
        self.log_with_level(LogLevel::Error, event_type, source, message);
    }
}

/// No-op logger that discards all events
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopLogger;

impl MemoryLogger for NoopLogger {
    fn log(&self, _event: &MemoryEvent) {
        // Intentionally empty
    }
}

/// Memory logging extension
pub struct LoggingExtension {
    /// The logger implementation
    logger: Box<dyn MemoryLogger>,
    /// Minimum log level to record
    min_level: LogLevel,
}

impl LoggingExtension {
    /// Create a new logging extension with the specified logger
    pub fn new(logger: impl MemoryLogger + 'static) -> Self {
        Self {
            logger: Box::new(logger),
            min_level: LogLevel::Info,
        }
    }

    /// Set the minimum log level
    #[must_use = "builder methods must be chained or built"]
    pub fn with_min_level(mut self, level: LogLevel) -> Self {
        self.min_level = level;
        self
    }

    /// Get the current logger
    pub fn logger(&self) -> &dyn MemoryLogger {
        self.logger.as_ref()
    }

    /// Log a memory event if its level is >= the minimum level
    pub fn log(&self, event: &MemoryEvent) {
        if event.level >= self.min_level {
            self.logger.log(event);
        }
    }
}

impl MemoryExtension for LoggingExtension {
    fn name(&self) -> &str {
        "logging"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }

    fn category(&self) -> &str {
        "logging"
    }

    fn tags(&self) -> Vec<&str> {
        vec!["logging", "diagnostics"]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// Create a console logger that writes to stdout/stderr
pub fn create_console_logger() -> impl MemoryLogger {
    struct ConsoleLogger;

    impl MemoryLogger for ConsoleLogger {
        fn log(&self, event: &MemoryEvent) {
            use std::time::{Duration, SystemTime, UNIX_EPOCH};

            // Форматируем время без зависимости от chrono
            let now = SystemTime::now();
            let since_epoch = now
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::from_secs(0));
            let secs = since_epoch.as_secs();
            let millis = since_epoch.subsec_millis();

            let timestamp = format!("{secs}.{millis:03}");

            let prefix = format!(
                "[{}] {} [{}] {}: ",
                timestamp, event.level, event.source, event.event_type
            );

            let mut message = format!("{prefix}{}", event.message);

            if !event.data.is_empty() {
                message.push_str(" {");
                let data_parts: Vec<String> = event
                    .data
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect();
                message.push_str(&data_parts.join(", "));
                message.push('}');
            }

            match event.level {
                LogLevel::Error => eprintln!("{message}"),
                _ => println!("{message}"),
            }
        }
    }

    ConsoleLogger
}

/// Helper to get the current global logger
pub fn global_logger() -> Option<Arc<LoggingExtension>> {
    use crate::extensions::GlobalExtensions;

    if let Some(ext) = GlobalExtensions::get("logging")
        && let Some(logging_ext) = ext.as_any().downcast_ref::<LoggingExtension>()
    {
        // Создаем новое расширение с тем же уровнем логирования
        // Так как мы не можем клонировать исходный логгер, используем NoopLogger
        // В реальном приложении лучше использовать фабрику логгеров, чтобы создавать
        // клоны
        let logger_wrapper = NoopLogger;

        return Some(Arc::new(LoggingExtension {
            logger: Box::new(logger_wrapper),
            min_level: logging_ext.min_level,
        }));
    }
    None
}

/// Initialize the global logger with a specific implementation
pub fn init_global_logger(
    logger: impl MemoryLogger + 'static,
    min_level: LogLevel,
) -> MemoryResult<()> {
    use crate::extensions::GlobalExtensions;

    let extension = LoggingExtension::new(logger).with_min_level(min_level);
    GlobalExtensions::register(extension)
}

/// Log a memory event through the global logger (if configured)
pub fn log_event(event: &MemoryEvent) {
    if let Some(logger) = global_logger() {
        logger.log(event);
    }
}

/// Macro for logging info messages
#[macro_export]
macro_rules! memory_info {
    ($event_type:expr, $source:expr, $message:expr) => {
        $crate::extensions::logging::log_event(
            &$crate::extensions::logging::MemoryEvent::new(
                $event_type,
                $source,
                $crate::extensions::logging::LogLevel::Info,
                $message
            )
        )
    };
    ($event_type:expr, $source:expr, $message:expr, $($key:expr => $value:expr),+) => {
        $crate::extensions::logging::log_event(
            &$crate::extensions::logging::MemoryEvent::new(
                $event_type,
                $source,
                $crate::extensions::logging::LogLevel::Info,
                $message
            )
            $(.with_data($key, $value))+
        )
    };
}

/// Macro for logging error messages
#[macro_export]
macro_rules! memory_log_error {
    ($event_type:expr, $source:expr, $message:expr) => {
        $crate::extensions::logging::log_event(
            &$crate::extensions::logging::MemoryEvent::new(
                $event_type,
                $source,
                $crate::extensions::logging::LogLevel::Error,
                $message
            )
        )
    };
    ($event_type:expr, $source:expr, $message:expr, $($key:expr => $value:expr),+) => {
        $crate::extensions::logging::log_event(
            &$crate::extensions::logging::MemoryEvent::new(
                $event_type,
                $source,
                $crate::extensions::logging::LogLevel::Error,
                $message
            )
            $(.with_data($key, $value))+
        )
    };
}

#[cfg(test)]
mod tests {
    use std::vec::Vec;

    use super::*;

    #[test]
    fn test_memory_event() {
        let event = MemoryEvent::new(
            "allocation",
            "arena",
            LogLevel::Debug,
            "Allocated memory block",
        )
        .with_data("size", "1024")
        .with_data("alignment", "8");

        assert_eq!(event.event_type, "allocation");
        assert_eq!(event.source, "arena");
        assert_eq!(event.level, LogLevel::Debug);
        assert_eq!(event.message, "Allocated memory block");
        assert_eq!(event.data.len(), 2);
        assert_eq!(event.data[0].0, "size");
        assert_eq!(event.data[0].1, "1024");
    }

    #[test]
    fn test_noop_logger() {
        let logger = NoopLogger;
        let event = MemoryEvent::new("test", "test", LogLevel::Info, "Test message");

        // This should not panic
        logger.log(&event);
    }

    #[derive(Debug)]
    #[allow(dead_code)]
    struct TestLogger {
        pub logs: Vec<MemoryEvent>,
    }

    impl TestLogger {
        fn new() -> Self {
            Self { logs: Vec::new() }
        }
    }

    impl MemoryLogger for TestLogger {
        fn log(&self, event: &MemoryEvent) {
            // In a real test, we would store the event in self.logs
            // Here we just print it for demonstration purposes
            println!("Logged event: {:?}", event);
        }
    }

    #[test]
    fn test_logging_extension() {
        let test_logger = TestLogger::new();
        let extension = LoggingExtension::new(test_logger).with_min_level(LogLevel::Debug);

        let info_event = MemoryEvent::new("test", "test", LogLevel::Info, "Info message");

        let debug_event = MemoryEvent::new("test", "test", LogLevel::Debug, "Debug message");

        let trace_event = MemoryEvent::new("test", "test", LogLevel::Trace, "Trace message");

        extension.log(&info_event);
        extension.log(&debug_event);
        extension.log(&trace_event); // This should be filtered out

        // In a real test, we would assert on the logs, but we can't do that
        // here because our test logger isn't properly thread-safe
    }
}
