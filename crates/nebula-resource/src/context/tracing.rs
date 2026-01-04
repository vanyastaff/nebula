//! Distributed tracing utilities and integrations

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::{ExecutionContext, TracingContext};

/// Span represents a unit of work in distributed tracing
#[derive(Debug, Clone)]
pub struct Span {
    /// Span identifier
    pub span_id: String,
    /// Trace identifier
    pub trace_id: String,
    /// Parent span identifier
    pub parent_span_id: Option<String>,
    /// Operation name
    pub operation_name: String,
    /// Span start time
    pub start_time: Instant,
    /// Span duration (set when finished)
    pub duration: Option<Duration>,
    /// Span tags
    pub tags: HashMap<String, String>,
    /// Span logs
    pub logs: Vec<SpanLog>,
    /// Span status
    pub status: SpanStatus,
    /// Span context
    pub context: ExecutionContext,
}

impl Span {
    /// Create a new span
    pub fn new(operation_name: String, context: ExecutionContext) -> Self {
        Self {
            span_id: context.tracing.span_id.clone(),
            trace_id: context.tracing.trace_id.clone(),
            parent_span_id: context.tracing.parent_span_id.clone(),
            operation_name,
            start_time: Instant::now(),
            duration: None,
            tags: HashMap::new(),
            logs: Vec::new(),
            status: SpanStatus::Ok,
            context,
        }
    }

    /// Add a tag to the span
    pub fn set_tag(&mut self, key: String, value: String) {
        self.tags.insert(key, value);
    }

    /// Add multiple tags to the span
    pub fn set_tags(&mut self, tags: HashMap<String, String>) {
        self.tags.extend(tags);
    }

    /// Log an event in the span
    pub fn log(&mut self, level: LogLevel, message: String) {
        self.logs.push(SpanLog {
            timestamp: Instant::now(),
            level,
            message,
            fields: HashMap::new(),
        });
    }

    /// Log an event with additional fields
    pub fn log_with_fields(&mut self, level: LogLevel, message: String, fields: HashMap<String, String>) {
        self.logs.push(SpanLog {
            timestamp: Instant::now(),
            level,
            message,
            fields,
        });
    }

    /// Set span status
    pub fn set_status(&mut self, status: SpanStatus) {
        self.status = status;
    }

    /// Set span error
    pub fn set_error(&mut self, error: &dyn std::error::Error) {
        self.status = SpanStatus::Error;
        self.set_tag("error".to_string(), "true".to_string());
        self.set_tag("error.kind".to_string(), "error".to_string());
        self.set_tag("error.message".to_string(), error.to_string());
    }

    /// Finish the span
    pub fn finish(&mut self) {
        self.duration = Some(self.start_time.elapsed());
    }

    /// Check if span is finished
    pub fn is_finished(&self) -> bool {
        self.duration.is_some()
    }

    /// Get span duration
    pub fn get_duration(&self) -> Option<Duration> {
        self.duration.or_else(|| Some(self.start_time.elapsed()))
    }
}

/// Span log entry
#[derive(Debug, Clone)]
pub struct SpanLog {
    /// Log timestamp
    pub timestamp: Instant,
    /// Log level
    pub level: LogLevel,
    /// Log message
    pub message: String,
    /// Additional fields
    pub fields: HashMap<String, String>,
}

/// Log level for span logs
#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    /// Trace level
    Trace,
    /// Debug level
    Debug,
    /// Info level
    Info,
    /// Warning level
    Warn,
    /// Error level
    Error,
}

/// Span status
#[derive(Debug, Clone, PartialEq)]
pub enum SpanStatus {
    /// Operation completed successfully
    Ok,
    /// Operation was cancelled
    Cancelled,
    /// Operation failed with an error
    Error,
    /// Operation timed out
    Timeout,
}

/// Tracer for creating and managing spans
pub struct Tracer {
    /// Active spans by span ID
    active_spans: parking_lot::RwLock<HashMap<String, Arc<parking_lot::Mutex<Span>>>>,
    /// Span reporters
    reporters: Vec<Arc<dyn SpanReporter + Send + Sync>>,
    /// Sampling configuration
    sampler: Arc<dyn Sampler + Send + Sync>,
}

impl Tracer {
    /// Create a new tracer
    pub fn new() -> Self {
        Self {
            active_spans: parking_lot::RwLock::new(HashMap::new()),
            reporters: Vec::new(),
            sampler: Arc::new(AlwaysSampler),
        }
    }

    /// Add a span reporter
    pub fn add_reporter(&mut self, reporter: Arc<dyn SpanReporter + Send + Sync>) {
        self.reporters.push(reporter);
    }

    /// Set the sampler
    pub fn set_sampler(&mut self, sampler: Arc<dyn Sampler + Send + Sync>) {
        self.sampler = sampler;
    }

    /// Start a new span
    pub fn start_span(&self, operation_name: String, context: ExecutionContext) -> TracingSpan {
        let should_sample = self.sampler.should_sample(&context, &operation_name);

        let span = Arc::new(parking_lot::Mutex::new(Span::new(operation_name, context.clone())));

        if should_sample {
            let span_id = span.lock().span_id.clone();
            self.active_spans.write().insert(span_id, span.clone());
        }

        TracingSpan {
            span,
            tracer: self,
            sampled: should_sample,
        }
    }

    /// Get an active span by ID
    pub fn get_span(&self, span_id: &str) -> Option<Arc<parking_lot::Mutex<Span>>> {
        self.active_spans.read().get(span_id).cloned()
    }

    /// Finish a span
    fn finish_span(&self, span: Arc<parking_lot::Mutex<Span>>) {
        let span_id = {
            let mut span_guard = span.lock();
            span_guard.finish();
            span_guard.span_id.clone()
        };

        // Remove from active spans
        self.active_spans.write().remove(&span_id);

        // Report the span
        for reporter in &self.reporters {
            let span_data = span.lock().clone();
            reporter.report_span(span_data);
        }
    }

    /// Get all active spans
    pub fn active_spans(&self) -> Vec<String> {
        self.active_spans.read().keys().cloned().collect()
    }
}

/// RAII wrapper for spans that automatically finishes on drop
pub struct TracingSpan<'a> {
    span: Arc<parking_lot::Mutex<Span>>,
    tracer: &'a Tracer,
    sampled: bool,
}

impl<'a> TracingSpan<'a> {
    /// Set a tag on the span
    pub fn set_tag(&self, key: String, value: String) {
        if self.sampled {
            self.span.lock().set_tag(key, value);
        }
    }

    /// Set multiple tags on the span
    pub fn set_tags(&self, tags: HashMap<String, String>) {
        if self.sampled {
            self.span.lock().set_tags(tags);
        }
    }

    /// Log an event
    pub fn log(&self, level: LogLevel, message: String) {
        if self.sampled {
            self.span.lock().log(level, message);
        }
    }

    /// Log an event with fields
    pub fn log_with_fields(&self, level: LogLevel, message: String, fields: HashMap<String, String>) {
        if self.sampled {
            self.span.lock().log_with_fields(level, message, fields);
        }
    }

    /// Set span status
    pub fn set_status(&self, status: SpanStatus) {
        if self.sampled {
            self.span.lock().set_status(status);
        }
    }

    /// Set span error
    pub fn set_error(&self, error: &dyn std::error::Error) {
        if self.sampled {
            self.span.lock().set_error(error);
        }
    }

    /// Get span ID
    pub fn span_id(&self) -> String {
        self.span.lock().span_id.clone()
    }

    /// Get trace ID
    pub fn trace_id(&self) -> String {
        self.span.lock().trace_id.clone()
    }

    /// Check if span is sampled
    pub fn is_sampled(&self) -> bool {
        self.sampled
    }

    /// Manually finish the span
    pub fn finish(self) {
        // Drop will handle the finishing
    }
}

impl<'a> Drop for TracingSpan<'a> {
    fn drop(&mut self) {
        if self.sampled {
            self.tracer.finish_span(self.span.clone());
        }
    }
}

/// Trait for reporting spans to external systems
pub trait SpanReporter {
    /// Report a finished span
    fn report_span(&self, span: Span);
}

/// Console span reporter for debugging
pub struct ConsoleSpanReporter;

impl SpanReporter for ConsoleSpanReporter {
    fn report_span(&self, span: Span) {
        println!(
            "SPAN: {} [{}] {} - {}ms - {}",
            span.operation_name,
            span.span_id,
            span.trace_id,
            span.get_duration().unwrap_or_default().as_millis(),
            match span.status {
                SpanStatus::Ok => "OK",
                SpanStatus::Error => "ERROR",
                SpanStatus::Cancelled => "CANCELLED",
                SpanStatus::Timeout => "TIMEOUT",
            }
        );

        for (key, value) in &span.tags {
            println!("  Tag: {} = {}", key, value);
        }

        for log in &span.logs {
            println!("  Log: {:?} - {}", log.level, log.message);
        }
    }
}

/// In-memory span reporter for testing
pub struct MemorySpanReporter {
    spans: parking_lot::Mutex<Vec<Span>>,
}

impl MemorySpanReporter {
    /// Create a new memory span reporter
    pub fn new() -> Self {
        Self {
            spans: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Get all reported spans
    pub fn get_spans(&self) -> Vec<Span> {
        self.spans.lock().clone()
    }

    /// Clear all spans
    pub fn clear(&self) {
        self.spans.lock().clear();
    }

    /// Get span count
    pub fn span_count(&self) -> usize {
        self.spans.lock().len()
    }
}

impl SpanReporter for MemorySpanReporter {
    fn report_span(&self, span: Span) {
        self.spans.lock().push(span);
    }
}

/// Trait for sampling decisions
pub trait Sampler {
    /// Decide whether to sample a span
    fn should_sample(&self, context: &ExecutionContext, operation_name: &str) -> bool;
}

/// Always sample spans
pub struct AlwaysSampler;

impl Sampler for AlwaysSampler {
    fn should_sample(&self, _context: &ExecutionContext, _operation_name: &str) -> bool {
        true
    }
}

/// Never sample spans
pub struct NeverSampler;

impl Sampler for NeverSampler {
    fn should_sample(&self, _context: &ExecutionContext, _operation_name: &str) -> bool {
        false
    }
}

/// Probabilistic sampler
pub struct ProbabilisticSampler {
    sample_rate: f64,
}

impl ProbabilisticSampler {
    /// Create a new probabilistic sampler
    pub fn new(sample_rate: f64) -> Self {
        Self {
            sample_rate: sample_rate.clamp(0.0, 1.0),
        }
    }
}

impl Sampler for ProbabilisticSampler {
    fn should_sample(&self, _context: &ExecutionContext, _operation_name: &str) -> bool {
        rand::random::<f64>() < self.sample_rate
    }
}

/// Rate limiting sampler
pub struct RateLimitSampler {
    max_traces_per_second: f64,
    last_sample: parking_lot::Mutex<Instant>,
    sample_interval: Duration,
}

impl RateLimitSampler {
    /// Create a new rate limiting sampler
    pub fn new(max_traces_per_second: f64) -> Self {
        let sample_interval = Duration::from_secs_f64(1.0 / max_traces_per_second.max(0.001));

        Self {
            max_traces_per_second,
            last_sample: parking_lot::Mutex::new(Instant::now() - sample_interval),
            sample_interval,
        }
    }
}

impl Sampler for RateLimitSampler {
    fn should_sample(&self, _context: &ExecutionContext, _operation_name: &str) -> bool {
        let mut last_sample = self.last_sample.lock();
        let now = Instant::now();

        if now.duration_since(*last_sample) >= self.sample_interval {
            *last_sample = now;
            true
        } else {
            false
        }
    }
}

/// Adaptive sampler that adjusts based on span volume
pub struct AdaptiveSampler {
    target_samples_per_second: f64,
    window_duration: Duration,
    sample_counts: parking_lot::Mutex<Vec<(Instant, usize)>>,
    current_rate: parking_lot::Mutex<f64>,
}

impl AdaptiveSampler {
    /// Create a new adaptive sampler
    pub fn new(target_samples_per_second: f64) -> Self {
        Self {
            target_samples_per_second,
            window_duration: Duration::from_secs(60),
            sample_counts: parking_lot::Mutex::new(Vec::new()),
            current_rate: parking_lot::Mutex::new(1.0),
        }
    }

    /// Update sampling rate based on recent activity
    fn update_sampling_rate(&self) {
        let now = Instant::now();
        let mut sample_counts = self.sample_counts.lock();

        // Remove old entries
        sample_counts.retain(|(timestamp, _)| now.duration_since(*timestamp) < self.window_duration);

        // Calculate current samples per second
        let total_samples: usize = sample_counts.iter().map(|(_, count)| count).sum();
        let window_seconds = self.window_duration.as_secs_f64();
        let current_samples_per_second = total_samples as f64 / window_seconds;

        // Adjust sampling rate
        let mut current_rate = self.current_rate.lock();
        if current_samples_per_second > self.target_samples_per_second {
            *current_rate *= 0.9; // Reduce sampling
        } else if current_samples_per_second < self.target_samples_per_second * 0.8 {
            *current_rate *= 1.1; // Increase sampling
        }

        *current_rate = current_rate.clamp(0.001, 1.0);
    }
}

impl Sampler for AdaptiveSampler {
    fn should_sample(&self, _context: &ExecutionContext, _operation_name: &str) -> bool {
        self.update_sampling_rate();

        let current_rate = *self.current_rate.lock();
        let should_sample = rand::random::<f64>() < current_rate;

        if should_sample {
            let now = Instant::now();
            self.sample_counts.lock().push((now, 1));
        }

        should_sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{IdentityContext, TenantContext};

    fn create_test_context() -> ExecutionContext {
        let identity = IdentityContext::system();
        let tenant = TenantContext::default_tenant();
        ExecutionContext::new(
            "test_workflow".to_string(),
            "test_execution".to_string(),
            "test_action".to_string(),
            identity,
            tenant,
        )
    }

    #[test]
    fn test_span_creation() {
        let context = create_test_context();
        let mut span = Span::new("test_operation".to_string(), context.clone());

        assert_eq!(span.operation_name, "test_operation");
        assert_eq!(span.trace_id, context.tracing.trace_id);
        assert_eq!(span.status, SpanStatus::Ok);
        assert!(!span.is_finished());

        span.set_tag("key".to_string(), "value".to_string());
        assert_eq!(span.tags.get("key"), Some(&"value".to_string()));

        span.log(LogLevel::Info, "Test log message".to_string());
        assert_eq!(span.logs.len(), 1);
        assert_eq!(span.logs[0].message, "Test log message");

        span.finish();
        assert!(span.is_finished());
    }

    #[test]
    fn test_tracer_span_management() {
        let mut tracer = Tracer::new();
        let reporter = Arc::new(MemorySpanReporter::new());
        tracer.add_reporter(reporter.clone());

        let context = create_test_context();

        {
            let span = tracer.start_span("test_operation".to_string(), context);
            let span_id = span.span_id();

            assert!(tracer.active_spans().contains(&span_id));
            span.set_tag("test".to_string(), "value".to_string());
        } // Span drops here and should be finished

        assert_eq!(reporter.span_count(), 1);
        let spans = reporter.get_spans();
        assert_eq!(spans[0].operation_name, "test_operation");
        assert!(spans[0].is_finished());
    }

    #[test]
    fn test_probabilistic_sampler() {
        let sampler = ProbabilisticSampler::new(0.5);
        let context = create_test_context();

        // Run multiple times to test probability
        let mut sampled_count = 0;
        for _ in 0..1000 {
            if sampler.should_sample(&context, "test") {
                sampled_count += 1;
            }
        }

        // Should be approximately 50% with some tolerance
        assert!(sampled_count > 400 && sampled_count < 600);
    }

    #[test]
    fn test_rate_limit_sampler() {
        let sampler = RateLimitSampler::new(2.0); // 2 samples per second
        let context = create_test_context();

        // First sample should be allowed
        assert!(sampler.should_sample(&context, "test"));

        // Second sample immediately should be rejected
        assert!(!sampler.should_sample(&context, "test"));

        // Wait for the sampling interval and try again
        std::thread::sleep(Duration::from_millis(600));
        assert!(sampler.should_sample(&context, "test"));
    }
}