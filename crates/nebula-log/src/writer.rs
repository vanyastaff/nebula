//! Writer implementations

use crate::{config::{WriterConfig, Rolling}, Result};
use std::io::{self, Write};
use tracing_subscriber::fmt::writer::BoxMakeWriter;

// Define a type alias for the return type of make_writer
// This allows us to handle the conditional compilation cleanly
#[cfg(feature = "file")]
type WriterGuards = Vec<tracing_appender::non_blocking::WorkerGuard>;

#[cfg(not(feature = "file"))]
type WriterGuards = Vec<()>;

/// Create a writer from configuration
pub fn make_writer(
    config: &WriterConfig,
) -> Result<(BoxMakeWriter, WriterGuards)> {
    #[cfg(feature = "file")]
    let mut guards = Vec::new();
    
    #[cfg(not(feature = "file"))]
    let guards = Vec::new();

    let writer: BoxMakeWriter = match config {
        WriterConfig::Stderr => BoxMakeWriter::new(io::stderr),
        WriterConfig::Stdout => BoxMakeWriter::new(io::stdout),

        #[cfg(feature = "file")]
        WriterConfig::File { path, rolling, non_blocking } => {
            let appender = match rolling {
                Some(Rolling::Hourly) => {
                    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                    let prefix = path.file_name().unwrap();
                    tracing_appender::rolling::hourly(dir, prefix)
                }
                Some(Rolling::Daily) => {
                    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                    let prefix = path.file_name().unwrap();
                    tracing_appender::rolling::daily(dir, prefix)
                }
                Some(Rolling::Size(_)) => {
                    return Err(anyhow::anyhow!(
                        "Size-based rolling is not yet implemented. Use Daily or Hourly."
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
            // TODO: Implement proper multi-writer
            if writers.is_empty() {
                return Err(anyhow::anyhow!("Multi writer needs at least one writer"));
            }
            return make_writer(&writers[0]);
        }
    };

    Ok((writer, guards))
}

/// Split writer that sends errors/warnings to stderr, rest to stdout
pub struct SplitStdWriter;

impl SplitStdWriter {
    pub fn make_writer() -> BoxMakeWriter {
        BoxMakeWriter::new(Self)
    }
}

impl<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> for SplitStdWriter {
    type Writer = Box<dyn Write + Send + 'a>;

    fn make_writer(&'a self) -> Self::Writer {
        Box::new(io::stderr())
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        use tracing::Level;
        match *meta.level() {
            Level::ERROR | Level::WARN => Box::new(io::stderr()),
            _ => Box::new(io::stdout()),
        }
    }
}