//! Telemetry integration setup (Sentry, log bridge)

/// Initialize telemetry integrations (Sentry + log bridge)
///
/// This handles:
/// - Sentry initialization and guard storage
///
/// Note: the `log` crate bridge (`tracing_log::LogTracer`) is wired up
/// automatically by `SubscriberInitExt::try_init()` via the `tracing-log`
/// feature of `tracing-subscriber`, so it must **not** be initialised here
/// to avoid a double-init conflict.
///
/// # Feature flags
/// - `sentry`: Enables Sentry integration
pub(super) fn init_telemetry(#[allow(unused_variables)] inner: &mut super::Inner) {
    // Initialize Sentry if enabled
    #[cfg(feature = "sentry")]
    {
        if let Some(guard) = crate::telemetry::sentry::init() {
            inner.sentry_guard = Some(guard);
        }
    }
}

/// Macro to push a Sentry layer into the layers Vec if feature is enabled
#[macro_export]
macro_rules! attach_sentry {
    ($layers:expr) => {{
        #[cfg(feature = "sentry")]
        {
            $layers.push(Box::new(sentry_tracing::layer().event_filter(|md| {
                use sentry_tracing::EventFilter;
                match *md.level() {
                    tracing::Level::ERROR => EventFilter::Event,
                    tracing::Level::WARN => EventFilter::Breadcrumb,
                    _ => EventFilter::Ignore,
                }
            })));
        }
    }};
}
