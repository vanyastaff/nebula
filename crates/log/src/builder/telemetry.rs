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
#[cfg_attr(
    not(feature = "sentry"),
    expect(
        clippy::needless_pass_by_ref_mut,
        reason = "only the sentry integration mutates the inner state"
    )
)]
pub(super) fn init_telemetry(
    #[cfg_attr(
        not(feature = "sentry"),
        expect(
            unused_variables,
            reason = "only the sentry integration reads the inner state"
        )
    )]
    inner: &mut super::Inner,
) {
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
            // sentry-tracing 0.49 splits the legacy combined layer: spans at
            // INFO and above become transactions, WARN events become
            // breadcrumbs and ERROR events become Sentry events; lower-level
            // events are not sent.
            use tracing_subscriber::{Layer as _, filter::LevelFilter};
            $layers.push(Box::new(
                sentry_tracing::SpanLayer::new().with_filter(LevelFilter::INFO),
            ));
            $layers.push(Box::new(
                sentry_tracing::BreadcrumbLayer::new().with_filter(
                    tracing_subscriber::filter::filter_fn(|md| *md.level() == tracing::Level::WARN),
                ),
            ));
            $layers.push(Box::new(
                sentry_tracing::ErrorLayer::new().with_filter(LevelFilter::ERROR),
            ));
        }
    }};
}
