//! Writer implementations

// Standard library
use std::io;

// External dependencies
use tracing_subscriber::fmt::writer::BoxMakeWriter;

// Internal crates
#[cfg(feature = "file")]
use crate::config::Rolling;
use crate::config::WriterConfig;
use crate::core::LogResult;

// Define a type alias for the return type of make_writer
// This allows us to handle the conditional compilation cleanly
#[cfg(feature = "file")]
type WriterGuards = Vec<tracing_appender::non_blocking::WorkerGuard>;

#[cfg(not(feature = "file"))]
type WriterGuards = Vec<()>;

/// Create a writer from configuration
pub fn make_writer(config: &WriterConfig) -> LogResult<(BoxMakeWriter, WriterGuards)> {
    #[cfg(feature = "file")]
    let mut guards = Vec::new();

    #[cfg(not(feature = "file"))]
    let guards = Vec::new();

    let writer: BoxMakeWriter = match config {
        WriterConfig::Stderr => BoxMakeWriter::new(io::stderr),
        WriterConfig::Stdout => BoxMakeWriter::new(io::stdout),

        #[cfg(feature = "file")]
        WriterConfig::File {
            path,
            rolling,
            non_blocking,
        } => {
            let appender = match rolling {
                Some(Rolling::Hourly) => {
                    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                    let prefix = path.file_name().ok_or_else(|| {
                        use crate::core::LogError;
                        nebula_error::NebulaError::log_config_error(format!(
                            "Invalid file path (no filename): '{}'",
                            path.display()
                        ))
                    })?;
                    tracing_appender::rolling::hourly(dir, prefix)
                }
                Some(Rolling::Daily) => {
                    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                    let prefix = path.file_name().ok_or_else(|| {
                        use crate::core::LogError;
                        nebula_error::NebulaError::log_config_error(format!(
                            "Invalid file path (no filename): '{}'",
                            path.display()
                        ))
                    })?;
                    tracing_appender::rolling::daily(dir, prefix)
                }
                Some(Rolling::Size(_)) => {
                    use crate::core::LogError;
                    return Err(nebula_error::NebulaError::log_config_error(
                        "Size-based rolling is not yet implemented. Use Daily or Hourly.",
                    ));
                }
                _ => tracing_appender::rolling::never(".", path),
            };

            if *non_blocking {
                let (non_blocking, guard) = tracing_appender::non_blocking(appender);
                guards.push(guard);
                BoxMakeWriter::new(non_blocking)
            } else {
                BoxMakeWriter::new(appender)
            }
        }

        WriterConfig::Multi(writers) => {
            // For now, use the first writer
            // TODO(feature): Implement proper multi-writer with fanout or tee functionality
            if writers.is_empty() {
                use crate::core::LogError;
                return Err(nebula_error::NebulaError::log_config_error(
                    "Multi writer needs at least one writer",
                ));
            }
            return make_writer(&writers[0]);
        }
    };

    Ok((writer, guards))
}
