//! Format layer creation macros

/// Macro to reduce duplication in format layer creation
///
/// This macro generates the format layer with all common configuration.
#[macro_export]
macro_rules! create_fmt_layer {
    ($format:ident, $display:expr, $writer:expr) => {{
        let mut layer = tracing_subscriber::fmt::layer()
            .$format()
            .with_writer($writer)
            .with_ansi($display.colors)
            .with_target($display.target)
            .with_file($display.source)
            .with_line_number($display.source)
            .with_thread_ids($display.thread_ids)
            .with_thread_names($display.thread_names);

        layer = layer.with_timer($crate::format::make_timer(if $display.time {
            $display.time_format.as_deref()
        } else {
            None
        }));

        layer
    }};
}

/// Macro for JSON format (has additional options)
#[macro_export]
macro_rules! create_json_layer {
    ($display:expr, $writer:expr) => {{
        let mut layer = tracing_subscriber::fmt::layer()
            .json()
            .with_writer($writer)
            .with_current_span(true)
            .with_span_list($display.span_list)
            .flatten_event($display.flatten)
            .with_ansi($display.colors)
            .with_target($display.target)
            .with_file($display.source)
            .with_line_number($display.source)
            .with_thread_ids($display.thread_ids)
            .with_thread_names($display.thread_names);

        layer = layer.with_timer($crate::format::make_timer(if $display.time {
            $display.time_format.as_deref()
        } else {
            None
        }));

        layer
    }};
}
