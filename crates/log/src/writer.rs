//! Writer implementations

use std::io::{self, Write};
#[cfg(feature = "file")]
use std::path::{Path, PathBuf};
#[cfg(feature = "file")]
use std::sync::Arc;

#[cfg(feature = "file")]
use parking_lot::{Mutex, MutexGuard};
use smallvec::SmallVec;
use tracing_subscriber::fmt::writer::{BoxMakeWriter, MakeWriter};

#[cfg(feature = "file")]
use crate::config::Rolling;
use crate::config::{DestinationFailurePolicy, WriterConfig};
use crate::core::{LogError, LogResult};

#[cfg(feature = "file")]
type WriterGuards = Vec<tracing_appender::non_blocking::WorkerGuard>;

#[cfg(not(feature = "file"))]
type WriterGuards = Vec<()>;

#[cfg(feature = "file")]
struct SharedWriterMakeWriter {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

#[cfg(feature = "file")]
impl SharedWriterMakeWriter {
    fn new(writer: Box<dyn Write + Send>) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
        }
    }
}

#[cfg(feature = "file")]
struct SharedWriterGuard<'a> {
    guard: MutexGuard<'a, Box<dyn Write + Send>>,
}

#[cfg(feature = "file")]
impl Write for SharedWriterGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.guard.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.guard.flush()
    }
}

#[cfg(feature = "file")]
impl<'a> MakeWriter<'a> for SharedWriterMakeWriter {
    type Writer = SharedWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        let guard = self.writer.lock();
        SharedWriterGuard { guard }
    }
}

struct FanoutMakeWriter {
    policy: DestinationFailurePolicy,
    writers: Vec<BoxMakeWriter>,
}

/// Inline capacity for fanout writers — avoids heap allocation when
/// the number of destinations is <= 4 (covers stderr + file + extras).
/// `make_writer()` is called per log event by tracing-subscriber.
type FanoutVec<'a> = SmallVec<[Box<dyn Write + 'a>; 4]>;

struct FanoutWriter<'a> {
    policy: DestinationFailurePolicy,
    writers: FanoutVec<'a>,
}

impl<'a> MakeWriter<'a> for FanoutMakeWriter {
    type Writer = FanoutWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        let writers: FanoutVec<'a> = self.writers.iter().map(|w| w.make_writer()).collect();
        FanoutWriter {
            policy: self.policy,
            writers,
        }
    }
}

impl Write for FanoutWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.policy {
            DestinationFailurePolicy::FailFast => write_fail_fast(&mut self.writers, buf),
            DestinationFailurePolicy::BestEffort => write_best_effort(&mut self.writers, buf),
            DestinationFailurePolicy::PrimaryWithFallback => {
                write_primary_with_fallback(&mut self.writers, buf)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.policy {
            DestinationFailurePolicy::FailFast => flush_fail_fast(&mut self.writers),
            DestinationFailurePolicy::BestEffort => flush_best_effort(&mut self.writers),
            DestinationFailurePolicy::PrimaryWithFallback => {
                flush_primary_with_fallback(&mut self.writers)
            }
        }
    }
}

fn write_fail_fast(writers: &mut FanoutVec<'_>, buf: &[u8]) -> io::Result<usize> {
    if writers.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one writer is required",
        ));
    }
    for writer in writers.iter_mut() {
        writer.write_all(buf)?;
    }
    Ok(buf.len())
}

fn write_best_effort(writers: &mut FanoutVec<'_>, buf: &[u8]) -> io::Result<usize> {
    let mut first_err = None;
    let mut success = false;
    for writer in writers.iter_mut() {
        match writer.write_all(buf) {
            Ok(()) => success = true,
            Err(err) if first_err.is_none() => first_err = Some(err),
            Err(_) => {}
        }
    }

    if success {
        Ok(buf.len())
    } else {
        Err(first_err.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one writer is required",
            )
        }))
    }
}

fn write_primary_with_fallback(writers: &mut FanoutVec<'_>, buf: &[u8]) -> io::Result<usize> {
    let Some((primary, fallback)) = writers.split_first_mut() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one writer is required",
        ));
    };

    if primary.write_all(buf).is_ok() {
        return Ok(buf.len());
    }

    for writer in fallback.iter_mut() {
        if writer.write_all(buf).is_ok() {
            return Ok(buf.len());
        }
    }

    Err(io::Error::other(
        "all writers failed with primary_with_fallback policy",
    ))
}

fn flush_fail_fast(writers: &mut FanoutVec<'_>) -> io::Result<()> {
    if writers.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one writer is required",
        ));
    }
    for writer in writers.iter_mut() {
        writer.flush()?;
    }
    Ok(())
}

fn flush_best_effort(writers: &mut FanoutVec<'_>) -> io::Result<()> {
    let mut first_err = None;
    let mut success = false;
    for writer in writers.iter_mut() {
        match writer.flush() {
            Ok(()) => success = true,
            Err(err) if first_err.is_none() => first_err = Some(err),
            Err(_) => {}
        }
    }
    if success {
        Ok(())
    } else {
        Err(first_err.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one writer is required",
            )
        }))
    }
}

fn flush_primary_with_fallback(writers: &mut FanoutVec<'_>) -> io::Result<()> {
    let Some((primary, fallback)) = writers.split_first_mut() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one writer is required",
        ));
    };

    if primary.flush().is_ok() {
        return Ok(());
    }

    for writer in fallback.iter_mut() {
        if writer.flush().is_ok() {
            return Ok(());
        }
    }

    Err(io::Error::other(
        "all writers failed with primary_with_fallback policy",
    ))
}

#[cfg(feature = "file")]
struct SizeRollingAppender {
    path: PathBuf,
    max_bytes: u64,
    max_files: u32,
    file: std::fs::File,
    current_size: u64,
}

#[cfg(feature = "file")]
impl SizeRollingAppender {
    fn new(path: PathBuf, max_megabytes: u64, max_files: u32) -> io::Result<Self> {
        let max_bytes = max_megabytes
            .checked_mul(1024 * 1024)
            .filter(|&b| b > 0)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "size rolling limit must be between 1 and 17592186044415 MB",
                )
            })?;
        let max_files = max_files.max(1);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let current_size = file.metadata()?.len();

        Ok(Self {
            path,
            max_bytes,
            max_files,
            file,
            current_size,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;

        // Remove the oldest backup if at capacity
        let oldest = PathBuf::from(format!("{}.{}", self.path.display(), self.max_files));
        if oldest.exists() {
            std::fs::remove_file(&oldest)?;
        }

        // Shift existing backups: .N-1 → .N, ... .1 → .2
        for i in (1..self.max_files).rev() {
            let src = PathBuf::from(format!("{}.{i}", self.path.display()));
            let dst = PathBuf::from(format!("{}.{}", self.path.display(), i + 1));
            if src.exists() {
                std::fs::rename(&src, &dst)?;
            }
        }

        // Current → .1
        if self.path.exists() {
            let first_backup = PathBuf::from(format!("{}.1", self.path.display()));
            std::fs::rename(&self.path, &first_backup)?;
        }

        self.file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        self.current_size = 0;
        Ok(())
    }
}

#[cfg(feature = "file")]
impl Write for SizeRollingAppender {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.current_size > 0
            && self.current_size.saturating_add(buf.len() as u64) > self.max_bytes
        {
            self.rotate()?;
        }
        let written = self.file.write(buf)?;
        self.current_size = self.current_size.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(feature = "file")]
fn file_prefix(path: &Path) -> LogResult<&std::ffi::OsStr> {
    path.file_name().ok_or_else(|| {
        LogError::Config(format!(
            "Invalid file path (no filename): '{}'",
            path.display()
        ))
    })
}

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
            let appender: Box<dyn Write + Send> = match rolling {
                Some(Rolling::Hourly) => {
                    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                    Box::new(tracing_appender::rolling::hourly(dir, file_prefix(path)?))
                }
                Some(Rolling::Daily) => {
                    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                    Box::new(tracing_appender::rolling::daily(dir, file_prefix(path)?))
                }
                Some(Rolling::Size(megabytes)) => Box::new(
                    SizeRollingAppender::new(path.clone(), *megabytes, 1).map_err(|e| {
                        LogError::Io(format!("failed to create size rolling writer: {e}"))
                    })?,
                ),
                Some(Rolling::SizeRetain {
                    megabytes,
                    max_files,
                }) => Box::new(
                    SizeRollingAppender::new(path.clone(), *megabytes, *max_files).map_err(
                        |e| LogError::Io(format!("failed to create size rolling writer: {e}")),
                    )?,
                ),
                _ => Box::new(tracing_appender::rolling::never(".", path)),
            };

            if *non_blocking {
                let (non_blocking, guard) = tracing_appender::non_blocking(appender);
                guards.push(guard);
                BoxMakeWriter::new(non_blocking)
            } else {
                BoxMakeWriter::new(SharedWriterMakeWriter::new(appender))
            }
        }

        WriterConfig::Multi { policy, writers } => {
            if writers.is_empty() {
                return Err(LogError::Config(
                    "Multi writer needs at least one writer".to_string(),
                ));
            }

            let mut make_writers = Vec::with_capacity(writers.len());
            for entry in writers {
                let (writer, sub_guards) = make_writer(entry)?;
                #[cfg(feature = "file")]
                guards.extend(sub_guards);
                #[cfg(not(feature = "file"))]
                let _ = sub_guards;
                make_writers.push(writer);
            }

            BoxMakeWriter::new(FanoutMakeWriter {
                policy: *policy,
                writers: make_writers,
            })
        }
    };

    Ok((writer, guards))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MockWriter {
        fail: bool,
        bytes: usize,
    }

    impl Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.fail {
                return Err(io::Error::other("write failed"));
            }
            self.bytes += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::other("flush failed"));
            }
            Ok(())
        }
    }

    #[test]
    fn fail_fast_stops_on_first_error() {
        let mut writers: FanoutVec<'_> = SmallVec::from_vec(vec![
            Box::new(MockWriter {
                fail: true,
                ..Default::default()
            }) as Box<dyn Write>,
            Box::new(MockWriter::default()) as Box<dyn Write>,
        ]);
        let result = write_fail_fast(&mut writers, b"hello");
        assert!(result.is_err());
    }

    #[test]
    fn best_effort_succeeds_when_any_writer_succeeds() {
        let mut writers: FanoutVec<'_> = SmallVec::from_vec(vec![
            Box::new(MockWriter {
                fail: true,
                ..Default::default()
            }) as Box<dyn Write>,
            Box::new(MockWriter::default()) as Box<dyn Write>,
        ]);
        let result = write_best_effort(&mut writers, b"hello");
        assert!(result.is_ok());
    }

    #[test]
    fn primary_with_fallback_uses_secondary_when_primary_fails() {
        let mut writers: FanoutVec<'_> = SmallVec::from_vec(vec![
            Box::new(MockWriter {
                fail: true,
                ..Default::default()
            }) as Box<dyn Write>,
            Box::new(MockWriter::default()) as Box<dyn Write>,
        ]);
        let result = write_primary_with_fallback(&mut writers, b"hello");
        assert_eq!(result.expect("fallback should succeed"), 5);
    }

    #[cfg(feature = "file")]
    #[test]
    fn size_rolling_rejects_overflow() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.log");

        let result = SizeRollingAppender::new(path, u64::MAX, 1);

        match result {
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                assert!(e.to_string().contains("17592186044415"));
            }
            Err(e) => panic!("wrong error kind: {e}"),
            Ok(_) => panic!("should reject overflow"),
        }
    }

    #[cfg(feature = "file")]
    #[test]
    fn size_rolling_rejects_zero() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.log");

        let result = SizeRollingAppender::new(path, 0, 1);

        assert!(matches!(result, Err(e) if e.kind() == io::ErrorKind::InvalidInput));
    }
}
