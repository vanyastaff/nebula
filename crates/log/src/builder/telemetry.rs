//! Telemetry integration setup (Sentry, log bridge)

/// Initialize telemetry integrations (Sentry + log bridge)
///
/// This handles:
/// - Sentry initialization and guard storage
/// - log crate bridge setup (tracing_log)
///
/// # Feature flags
/// - `sentry`: Enables Sentry integration
/// - `log-compat`: Enables log crate bridge
pub(super) fn init_telemetry(#[allow(unused_variables)] inner: &mut super::Inner) {
    // Initialize Sentry if enabled
    #[cfg(feature = "sentry")]
    {
        if let Some(guard) = crate::telemetry::sentry::init() {
            inner.sentry_guard = Some(guard);
        }
    }

    // Bridge log crate if enabled
    #[cfg(feature = "log-compat")]
    {
        let _ = tracing_log::LogTracer::init();
    }
}

/// Macro to attach Sentry layer if feature is enabled
#[macro_export]
macro_rules! attach_sentry {
    ($subscriber:expr) => {{
        #[cfg(feature = "sentry")]
        {
            use tracing_subscriber::layer::SubscriberExt;
            $subscriber.with(sentry_tracing::layer().event_filter(|md| {
                use sentry_tracing::EventFilter;
                match *md.level() {
                    tracing::Level::ERROR => EventFilter::Event,
                    tracing::Level::WARN => EventFilter::Breadcrumb,
                    _ => EventFilter::Ignore,
                }
            }))
        }

        #[cfg(not(feature = "sentry"))]
        {
            $subscriber
        }
    }};
}
