//! Buffered execution recorder with configurable flush strategies.
//!
//! [`BufferedRecorder`] collects [`ResourceUsageRecord`] and [`CallRecord`]
//! entries via a non-blocking MPSC channel and flushes them in batches to a
//! pluggable [`RecordSink`]. This avoids blocking the hot path (action
//! execution) while ensuring records are eventually persisted or exported.
//!
//! ## Quick Start
//!
//! ```no_run
//! use nebula_telemetry::recorder::{BufferedRecorder, BufferedRecorderConfig, LogSink};
//!
//! # async fn example() {
//! let recorder = BufferedRecorder::start(
//!     BufferedRecorderConfig::default(),
//!     LogSink,
//! );
//! // Use as `Arc<dyn Recorder>` — records are buffered and flushed asynchronously.
//! # }
//! ```

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing;

use crate::trace::{CallRecord, Recorder, ResourceUsageRecord};

// ── RecordEntry ──────────────────────────────────────────────────────────────

/// A single buffered entry — either a usage record or a call record.
#[derive(Debug)]
#[non_exhaustive]
pub enum RecordEntry {
    /// Tier 1: automatic resource usage.
    Usage(ResourceUsageRecord),
    /// Tier 2: optional per-call enrichment (boxed to reduce enum size).
    Call(Box<CallRecord>),
}

// ── RecordSink trait ─────────────────────────────────────────────────────────

/// Pluggable flush destination for buffered records.
///
/// Implementors receive batches of records and persist or export them.
/// The flush method is async to support I/O-bound sinks (database, HTTP).
#[async_trait::async_trait]
pub trait RecordSink: Send + Sync + 'static {
    /// Flush a batch of records to the sink.
    ///
    /// Called periodically by the background task when records are available.
    /// Implementations should handle errors internally (log and continue).
    async fn flush(&self, records: Vec<RecordEntry>);
}

// ── LogSink ──────────────────────────────────────────────────────────────────

/// Default sink that writes records to `tracing::info!`.
///
/// Useful for development, debugging, and as a reference implementation.
#[derive(Debug, Clone, Copy)]
pub struct LogSink;

#[async_trait::async_trait]
impl RecordSink for LogSink {
    async fn flush(&self, records: Vec<RecordEntry>) {
        for entry in &records {
            match entry {
                RecordEntry::Usage(r) => {
                    tracing::info!(
                        resource = %r.resource_key,
                        wait_ms = r.wait_duration.as_millis() as u64,
                        hold_ms = r.hold_duration.as_millis() as u64,
                        drop_reason = ?r.drop_reason,
                        "resource usage recorded"
                    );
                }
                RecordEntry::Call(r) => {
                    tracing::info!(
                        resource = %r.resource_key,
                        operation = %r.operation,
                        duration_ms = r.duration.as_millis() as u64,
                        status = ?r.status,
                        "call recorded"
                    );
                }
            }
        }
    }
}

// ── BufferedRecorderConfig ───────────────────────────────────────────────────

/// Configuration for [`BufferedRecorder`].
#[derive(Debug, Clone)]
pub struct BufferedRecorderConfig {
    /// Maximum number of records buffered before a forced flush.
    ///
    /// Default: `1024`.
    pub buffer_size: usize,
    /// Interval between periodic flushes.
    ///
    /// Default: `5 seconds`.
    pub flush_interval: Duration,
}

impl Default for BufferedRecorderConfig {
    fn default() -> Self {
        Self {
            buffer_size: 1024,
            flush_interval: Duration::from_secs(5),
        }
    }
}

// ── BufferedRecorder ─────────────────────────────────────────────────────────

/// Buffered execution recorder that batches records and flushes asynchronously.
///
/// Records are submitted via the [`Recorder`] trait methods (`record_usage`,
/// `record_call`) using a non-blocking channel send. A background tokio task
/// collects entries and flushes them to a [`RecordSink`] when either the
/// buffer fills or the flush interval elapses.
///
/// On channel full, records are dropped with a warning log (back-pressure).
pub struct BufferedRecorder {
    sender: mpsc::Sender<RecordEntry>,
    shutdown: Option<ShutdownHandle>,
}

/// Handle for draining remaining records on shutdown.
struct ShutdownHandle {
    handle: tokio::task::JoinHandle<()>,
}

impl BufferedRecorder {
    /// Start the buffered recorder with the given config and sink.
    ///
    /// Spawns a background tokio task for batching and flushing.
    pub fn start<S: RecordSink>(config: BufferedRecorderConfig, sink: S) -> Self {
        let (tx, rx) = mpsc::channel(config.buffer_size);
        let sink = Arc::new(sink);

        tracing::debug!(
            buffer_size = config.buffer_size,
            flush_interval_ms = config.flush_interval.as_millis() as u64,
            "buffered recorder started"
        );

        let handle = tokio::spawn(flush_loop(rx, sink, config));

        Self {
            sender: tx,
            shutdown: Some(ShutdownHandle { handle }),
        }
    }

    /// Shut down the recorder: close the channel and drain remaining records.
    ///
    /// Returns after the background task has flushed all remaining entries.
    pub async fn shutdown(mut self) {
        // Drop sender to signal the background task to stop
        drop(self.sender.clone());
        // The real sender is still held — we need to take ownership
        if let Some(handle) = self.shutdown.take() {
            // Drop the sender field to close the channel
            drop(std::mem::replace(&mut self.sender, mpsc::channel(1).0));
            match handle.handle.await {
                Ok(()) => {
                    tracing::info!("buffered recorder shutdown complete");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "buffered recorder task panicked during shutdown");
                }
            }
        }
    }
}

impl Recorder for BufferedRecorder {
    fn record_usage(&self, record: ResourceUsageRecord) {
        if self.sender.try_send(RecordEntry::Usage(record)).is_err() {
            tracing::warn!("buffered recorder channel full, dropping usage record");
        }
    }

    fn record_call(&self, record: CallRecord) {
        if self
            .sender
            .try_send(RecordEntry::Call(Box::new(record)))
            .is_err()
        {
            tracing::warn!("buffered recorder channel full, dropping call record");
        }
    }
}

// ── Background flush loop ────────────────────────────────────────────────────

async fn flush_loop(
    mut rx: mpsc::Receiver<RecordEntry>,
    sink: Arc<dyn RecordSink>,
    config: BufferedRecorderConfig,
) {
    let mut buffer: Vec<RecordEntry> = Vec::with_capacity(config.buffer_size);
    let mut interval = tokio::time::interval(config.flush_interval);
    // First tick completes immediately — skip it
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !buffer.is_empty() {
                    let batch = std::mem::replace(
                        &mut buffer,
                        Vec::with_capacity(config.buffer_size),
                    );
                    tracing::debug!(count = batch.len(), "flushing buffered records (interval)");
                    sink.flush(batch).await;
                }
            }
            entry = rx.recv() => {
                match entry {
                    Some(record) => {
                        buffer.push(record);
                        if buffer.len() >= config.buffer_size {
                            let batch = std::mem::replace(
                                &mut buffer,
                                Vec::with_capacity(config.buffer_size),
                            );
                            tracing::debug!(count = batch.len(), "flushing buffered records (buffer full)");
                            sink.flush(batch).await;
                        }
                    }
                    None => {
                        // Channel closed — drain remaining and exit
                        if !buffer.is_empty() {
                            let count = buffer.len();
                            tracing::debug!(count, "flushing remaining buffered records on shutdown");
                            sink.flush(buffer).await;
                        }
                        return;
                    }
                }
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Instant;

    use nebula_core::ResourceKey;

    use super::*;
    use crate::trace::{CallStatus, DropReason};

    /// Test sink that collects flushed records for assertions.
    #[derive(Debug, Clone, Default)]
    struct CollectingSink {
        flushed: Arc<Mutex<Vec<RecordEntry>>>,
    }

    #[async_trait::async_trait]
    impl RecordSink for CollectingSink {
        async fn flush(&self, records: Vec<RecordEntry>) {
            self.flushed.lock().expect("lock poisoned").extend(records);
        }
    }

    fn make_usage_record(key: &str) -> ResourceUsageRecord {
        ResourceUsageRecord {
            resource_key: ResourceKey::new(key).expect("valid test key"),
            acquired_at: Instant::now(),
            wait_duration: Duration::from_millis(1),
            hold_duration: Duration::from_millis(10),
            drop_reason: DropReason::Released,
        }
    }

    fn make_call_record(key: &str, op: &str) -> CallRecord {
        CallRecord {
            resource_key: ResourceKey::new(key).expect("valid test key"),
            operation: op.to_owned(),
            started_at: Instant::now(),
            duration: Duration::from_millis(5),
            status: CallStatus::Success,
            request: None,
            response: None,
            metadata: HashMap::new(),
            trace_context: None,
        }
    }

    #[tokio::test]
    async fn records_are_flushed_after_interval() {
        let sink = CollectingSink::default();
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 100,
                flush_interval: Duration::from_millis(50),
            },
            sink.clone(),
        );

        recorder.record_usage(make_usage_record("db"));
        recorder.record_call(make_call_record("db", "SELECT"));

        // Wait for flush interval to trigger
        tokio::time::sleep(Duration::from_millis(150)).await;

        let flushed = sink.flushed.lock().unwrap();
        assert_eq!(flushed.len(), 2);
    }

    #[tokio::test]
    async fn records_are_flushed_when_buffer_fills() {
        let sink = CollectingSink::default();
        let buffer_size = 4;
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size,
                flush_interval: Duration::from_secs(60), // Long interval — won't trigger
            },
            sink.clone(),
        );

        for i in 0..buffer_size {
            recorder.record_usage(make_usage_record(&format!("res-{i}")));
        }

        // Give the background task time to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        let flushed = sink.flushed.lock().unwrap();
        assert_eq!(flushed.len(), buffer_size);
    }

    #[tokio::test]
    async fn back_pressure_drops_without_panic() {
        let sink = CollectingSink::default();
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 2,
                flush_interval: Duration::from_secs(60),
            },
            sink.clone(),
        );

        // Fill channel beyond capacity — should not panic
        for i in 0..100 {
            recorder.record_usage(make_usage_record(&format!("res-{i}")));
        }

        // No panic = success
    }

    #[tokio::test]
    async fn shutdown_drains_remaining_records() {
        let sink = CollectingSink::default();
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 100,
                flush_interval: Duration::from_secs(60),
            },
            sink.clone(),
        );

        recorder.record_usage(make_usage_record("db"));
        recorder.record_call(make_call_record("api", "POST /users"));

        // Give records time to enter channel
        tokio::time::sleep(Duration::from_millis(10)).await;

        recorder.shutdown().await;

        let flushed = sink.flushed.lock().unwrap();
        assert_eq!(flushed.len(), 2);
    }

    #[tokio::test]
    async fn log_sink_does_not_panic() {
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 10,
                flush_interval: Duration::from_millis(50),
            },
            LogSink,
        );

        recorder.record_usage(make_usage_record("http"));
        recorder.record_call(make_call_record("http", "GET /health"));

        tokio::time::sleep(Duration::from_millis(100)).await;
        // No panic = success
    }

    #[tokio::test]
    async fn multiple_concurrent_producers() {
        let sink = CollectingSink::default();
        let recorder = Arc::new(BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 1000,
                flush_interval: Duration::from_millis(50),
            },
            sink.clone(),
        ));

        let mut handles = Vec::new();
        for t in 0..4 {
            let rec = Arc::clone(&recorder);
            handles.push(tokio::spawn(async move {
                for i in 0..25 {
                    rec.record_usage(make_usage_record(&format!("t{t}-r{i}")));
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        // Wait for flush
        tokio::time::sleep(Duration::from_millis(150)).await;

        let flushed = sink.flushed.lock().unwrap();
        assert_eq!(flushed.len(), 100);
    }
}
