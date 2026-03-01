//! Format layer creation macros

/// Macro to reduce duplication in format layer creation.
///
/// Produces a `tracing_subscriber::fmt::Layer` with the given format and display
/// options. Timer type is `format::Timer` (our custom enum).
#[macro_export]
macro_rules! create_fmt_layer {
    ($format:ident, $display:expr, $writer:expr) => {{
        let timer = $crate::format::make_timer(if $display.time {
            $display.time_format.as_deref()
        } else {
            None
        });
        tracing_subscriber::fmt::layer()
            .$format()
            .with_writer($writer)
            .with_timer(timer)
            .with_ansi($display.colors)
            .with_target($display.target)
            .with_file($display.source)
            .with_line_number($display.source)
            .with_thread_ids($display.thread_ids)
            .with_thread_names($display.thread_names)
    }};
}

/// Macro for JSON format (has additional options).
#[macro_export]
macro_rules! create_json_layer {
    ($display:expr, $writer:expr) => {{
        let timer = $crate::format::make_timer(if $display.time {
            $display.time_format.as_deref()
        } else {
            None
        });
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer($writer)
            .with_timer(timer)
            .with_current_span(true)
            .with_span_list($display.span_list)
            .flatten_event($display.flatten)
            .with_ansi($display.colors)
            .with_target($display.target)
            .with_file($display.source)
            .with_line_number($display.source)
            .with_thread_ids($display.thread_ids)
            .with_thread_names($display.thread_names)
    }};
}

/// Macro for logfmt format (key=value pairs).
#[macro_export]
macro_rules! create_logfmt_layer {
    ($display:expr, $writer:expr) => {{ $crate::format::make_logfmt_layer($writer, $display) }};
}
