//! Format utilities (time, logfmt)

use std::fmt;

use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

// ---------------------------------------------------------------------------
// Timer
// ---------------------------------------------------------------------------

/// A timer that formats timestamps using a custom `time` crate format string,
/// or falls back to [`SystemTime`] when no format is specified.
pub enum Timer {
    /// Default system time output.
    System(SystemTime),
    /// Custom format via the `time` crate.
    Custom {
        /// Compiled format description (owned, init-time only).
        format: time::format_description::OwnedFormatItem,
    },
}

impl FormatTime for Timer {
    fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
        match self {
            Timer::System(s) => s.format_time(w),
            Timer::Custom { format } => {
                let now = time::OffsetDateTime::now_utc();
                let formatted = now.format(format).map_err(|_| fmt::Error)?;
                write!(w, "{formatted}")
            }
        }
    }
}

/// Create a timer from an optional format description string.
///
/// When `format` is `Some(desc)`, parses the description via
/// `time::format_description::parse_owned`. Returns [`Timer::System`] when
/// `None` or on parse failure (logs a warning).
pub fn make_timer(format: Option<&str>) -> Timer {
    match format {
        Some(desc) => match time::format_description::parse_owned::<2>(desc) {
            Ok(compiled) => Timer::Custom { format: compiled },
            Err(e) => {
                tracing::warn!(
                    format = desc,
                    error = %e,
                    "invalid time format, falling back to SystemTime"
                );
                Timer::System(SystemTime)
            }
        },
        None => Timer::System(SystemTime),
    }
}

// ---------------------------------------------------------------------------
// Logfmt formatter
// ---------------------------------------------------------------------------

/// A `tracing_subscriber` event formatter that outputs `key=value` logfmt lines.
///
/// Format: `ts=<RFC3339> level=<LEVEL> target=<target> msg=<message> <fields...>`
pub struct LogfmtFormatter {
    display_time: bool,
    display_target: bool,
    display_source: bool,
}

impl LogfmtFormatter {
    /// Create a new logfmt formatter with the given display options.
    pub const fn new(display_time: bool, display_target: bool, display_source: bool) -> Self {
        Self {
            display_time,
            display_target,
            display_source,
        }
    }
}

/// Visitor that collects event fields as logfmt `key=value` pairs.
struct LogfmtVisitor<'a> {
    writer: &'a mut dyn fmt::Write,
}

impl<'a> LogfmtVisitor<'a> {
    fn new(writer: &'a mut dyn fmt::Write) -> Self {
        Self { writer }
    }
}

/// Write a logfmt value, quoting if necessary.
fn write_logfmt_value(w: &mut dyn fmt::Write, key: &str, value: &str) -> fmt::Result {
    if value.contains(' ') || value.contains('"') || value.contains('=') {
        write!(w, " {key}=\"{}\"", value.replace('"', "\\\""))
    } else {
        write!(w, " {key}={value}")
    }
}

impl tracing::field::Visit for LogfmtVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        let key = if field.name() == "message" {
            "msg"
        } else {
            field.name()
        };
        let _ = write!(self.writer, " {key}={value:?}");
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let key = if field.name() == "message" {
            "msg"
        } else {
            field.name()
        };
        let _ = write_logfmt_value(self.writer, key, value);
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        let _ = write!(self.writer, " {}={value}", field.name());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        let _ = write!(self.writer, " {}={value}", field.name());
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        let _ = write!(self.writer, " {}={value}", field.name());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        let _ = write!(self.writer, " {}={value}", field.name());
    }
}

impl<S, N> FormatEvent<S, N> for LogfmtFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();

        // ts=<RFC3339>
        if self.display_time {
            let now = time::OffsetDateTime::now_utc();
            let ts = now
                .format(&time::format_description::well_known::Rfc3339)
                .map_err(|_| fmt::Error)?;
            write!(writer, "ts={ts}")?;
        }

        // level=<LEVEL>
        if self.display_time {
            write!(writer, " level={}", meta.level())?;
        } else {
            write!(writer, "level={}", meta.level())?;
        }

        // target=<target>
        if self.display_target {
            write!(writer, " target={}", meta.target())?;
        }

        // source=<file:line>
        if self.display_source
            && let (Some(file), Some(line)) = (meta.file(), meta.line())
        {
            write!(writer, " source={file}:{line}")?;
        }

        // Span context (key=value from parent spans)
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                let fields_str = span
                    .extensions()
                    .get::<tracing_subscriber::fmt::FormattedFields<N>>()
                    .map(|f| f.fields.trim().to_owned())
                    .unwrap_or_default();
                for pair in fields_str.split(", ").filter(|s| !s.is_empty()) {
                    write!(writer, " {pair}")?;
                }
            }
        }

        // Event fields
        let mut visitor = LogfmtVisitor::new(&mut writer);
        event.record(&mut visitor);

        writeln!(writer)
    }
}

/// Create a logfmt-based `fmt::Layer` ready for use in a tracing subscriber.
pub fn make_logfmt_layer<S, W>(
    writer: W,
    display: &crate::config::DisplayConfig,
) -> tracing_subscriber::fmt::Layer<
    S,
    tracing_subscriber::fmt::format::DefaultFields,
    LogfmtFormatter,
    W,
>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'writer> tracing_subscriber::fmt::writer::MakeWriter<'writer> + 'static,
{
    tracing_subscriber::fmt::layer()
        .event_format(LogfmtFormatter::new(
            display.time,
            display.target,
            display.source,
        ))
        .with_writer(writer)
        .with_ansi(false) // logfmt is never colored
}
