//! Convenience macros for structured logging

/// Time a block of code
#[macro_export]
macro_rules! timed {
    ($name:expr, $body:expr) => {{
        let _timer = $crate::TimerGuard::new($name);
        $body
    }};
}

/// Time an async block
#[macro_export]
macro_rules! async_timed {
    ($name:expr, $body:expr) => {{
        use $crate::Timed;
        async move { $body }.timed($name).await
    }};
}

/// Log an error and return it
#[macro_export]
macro_rules! log_error {
    ($err:expr) => {{
        let e = $err;
        $crate::error!(error = ?e);
        e
    }};
    ($err:expr, $($arg:tt)*) => {{
        let e = $err;
        $crate::error!(error = ?e, $($arg)*);
        e
    }};
}

/// Create a span with timing
#[macro_export]
macro_rules! timed_span {
    ($name:expr) => {
        tracing::info_span!($name, elapsed_ms = tracing::field::Empty)
    };
    ($level:expr, $name:expr) => {
        tracing::span!($level, $name, elapsed_ms = tracing::field::Empty)
    };
    ($level:expr, $name:expr, $($field:tt)*) => {
        tracing::span!($level, $name, elapsed_ms = tracing::field::Empty, $($field)*)
    };
}

/// Log and measure an async operation
#[macro_export]
macro_rules! measure {
    ($name:expr, $future:expr) => {{
        use tracing::Instrument;
        let __start = std::time::Instant::now();
        let span = $crate::timed_span!($name);
        let instrumented_future = async move {
            let result = $future.await;
            result
        }.instrument(span);

        let result = instrumented_future.await;
        let elapsed = __start.elapsed().as_millis();
        tracing::info!(name = %$name, elapsed_ms = elapsed, "Operation completed");
        result
    }};
}

/// Set context fields for current scope
#[macro_export]
macro_rules! with_context {
    ($($key:ident = $value:expr),* $(,)?) => {{
        let ctx = $crate::Context::current()
            $(.with_field(stringify!($key), $value))*;
        ctx.set_current()
    }};
}
