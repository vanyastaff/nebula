//! EventBus snapshot recording into the metrics registry.
//!
//! The four `NEBULA_EVENTBUS_*` gauges (sent / dropped / subscribers /
//! drop-ratio in ppm) form a single observation point: every scrape interval
//! the engine snapshots [`nebula_eventbus::EventBusStats`] and writes the four
//! values atomically into the registry. Per ADR-0046 the recording is exposed
//! as a single free function instead of a method on a bridge type — the
//! function takes a [`MetricsRegistry`] reference, applies the canonical
//! `nebula_*` names from [`crate::naming`], and clamps the platform-specific
//! integer widths down to the gauge's `i64` storage.
//!
//! ```rust
//! use nebula_eventbus::EventBusStats;
//! use nebula_metrics::{MetricsRegistry, record_eventbus_stats};
//!
//! let registry = MetricsRegistry::new();
//! let stats = EventBusStats {
//!     sent_count: 75,
//!     dropped_count: 25,
//!     subscriber_count: 3,
//! };
//! record_eventbus_stats(&registry, &stats).unwrap();
//! ```

use nebula_eventbus::EventBusStats;

use crate::{
    MetricsResult,
    naming::{
        NEBULA_EVENTBUS_DROP_RATIO_PPM, NEBULA_EVENTBUS_DROPPED, NEBULA_EVENTBUS_SENT,
        NEBULA_EVENTBUS_SUBSCRIBERS,
    },
    registry::MetricsRegistry,
};

/// Record an [`EventBusStats`] snapshot into the four `NEBULA_EVENTBUS_*` gauges.
///
/// `sent_count` / `dropped_count` are clamped from `u64` to `i64::MAX` on
/// overflow; `subscriber_count` is clamped from `usize` the same way. The drop
/// ratio is computed from the snapshot, scaled to parts-per-million, rounded
/// to the nearest integer, and clamped to `[0, i64::MAX]`. Non-finite ratios
/// (only possible if `EventBusStats::drop_ratio` ever returns `NaN` or `±∞`)
/// are recorded as zero.
pub fn record_eventbus_stats(
    registry: &MetricsRegistry,
    stats: &EventBusStats,
) -> MetricsResult<()> {
    registry
        .gauge(NEBULA_EVENTBUS_SENT)?
        .set(clamp_u64_to_i64(stats.sent_count));
    registry
        .gauge(NEBULA_EVENTBUS_DROPPED)?
        .set(clamp_u64_to_i64(stats.dropped_count));
    registry
        .gauge(NEBULA_EVENTBUS_SUBSCRIBERS)?
        .set(clamp_usize_to_i64(stats.subscriber_count));

    let ppm = (stats.drop_ratio() * 1_000_000.0).round();
    let ppm = if ppm.is_finite() {
        ppm.clamp(0.0, i64::MAX as f64) as i64
    } else {
        0
    };
    registry.gauge(NEBULA_EVENTBUS_DROP_RATIO_PPM)?.set(ppm);
    Ok(())
}

fn clamp_u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn clamp_usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use nebula_eventbus::EventBusStats;

    use super::record_eventbus_stats;
    use crate::{
        MetricsRegistry,
        naming::{
            NEBULA_EVENTBUS_DROP_RATIO_PPM, NEBULA_EVENTBUS_DROPPED, NEBULA_EVENTBUS_SENT,
            NEBULA_EVENTBUS_SUBSCRIBERS,
        },
    };

    #[test]
    fn record_eventbus_stats_writes_all_four_gauges() {
        let registry = MetricsRegistry::new();

        let stats = EventBusStats {
            sent_count: 75,
            dropped_count: 25,
            subscriber_count: 3,
        };

        record_eventbus_stats(&registry, &stats).unwrap();

        assert_eq!(registry.gauge(NEBULA_EVENTBUS_SENT).unwrap().get(), 75);
        assert_eq!(registry.gauge(NEBULA_EVENTBUS_DROPPED).unwrap().get(), 25);
        assert_eq!(
            registry.gauge(NEBULA_EVENTBUS_SUBSCRIBERS).unwrap().get(),
            3
        );
        assert_eq!(
            registry
                .gauge(NEBULA_EVENTBUS_DROP_RATIO_PPM)
                .unwrap()
                .get(),
            250_000
        );
    }

    #[test]
    fn record_eventbus_stats_handles_zero_totals_and_default_values() {
        let registry = MetricsRegistry::new();

        let stats = EventBusStats {
            sent_count: 0,
            dropped_count: 0,
            subscriber_count: 0,
        };
        record_eventbus_stats(&registry, &stats).unwrap();
        assert_eq!(registry.gauge(NEBULA_EVENTBUS_SENT).unwrap().get(), 0);
        assert_eq!(registry.gauge(NEBULA_EVENTBUS_DROPPED).unwrap().get(), 0);
        assert_eq!(
            registry.gauge(NEBULA_EVENTBUS_SUBSCRIBERS).unwrap().get(),
            0
        );
        assert_eq!(
            registry
                .gauge(NEBULA_EVENTBUS_DROP_RATIO_PPM)
                .unwrap()
                .get(),
            0
        );

        let default_stats = EventBusStats::default();
        record_eventbus_stats(&registry, &default_stats).unwrap();
        assert_eq!(registry.gauge(NEBULA_EVENTBUS_SENT).unwrap().get(), 0);
        assert_eq!(registry.gauge(NEBULA_EVENTBUS_DROPPED).unwrap().get(), 0);
        assert_eq!(
            registry.gauge(NEBULA_EVENTBUS_SUBSCRIBERS).unwrap().get(),
            0
        );
        assert_eq!(
            registry
                .gauge(NEBULA_EVENTBUS_DROP_RATIO_PPM)
                .unwrap()
                .get(),
            0
        );
    }

    #[test]
    fn record_eventbus_stats_handles_full_drop_ratio_and_rounding() {
        let registry = MetricsRegistry::new();

        let full_drop = EventBusStats {
            sent_count: 1_000_000,
            dropped_count: 1_000_000,
            subscriber_count: 42,
        };
        record_eventbus_stats(&registry, &full_drop).unwrap();
        assert_eq!(
            registry
                .gauge(NEBULA_EVENTBUS_DROP_RATIO_PPM)
                .unwrap()
                .get(),
            500_000
        );

        let fractional = EventBusStats {
            sent_count: 3,
            dropped_count: 1,
            subscriber_count: 1,
        };
        record_eventbus_stats(&registry, &fractional).unwrap();
        assert_eq!(
            registry
                .gauge(NEBULA_EVENTBUS_DROP_RATIO_PPM)
                .unwrap()
                .get(),
            250_000
        );
    }

    #[test]
    fn record_eventbus_stats_clamps_large_values_to_i64_max() {
        let registry = MetricsRegistry::new();

        let stats = EventBusStats {
            sent_count: u64::MAX,
            dropped_count: 0,
            subscriber_count: usize::MAX,
        };
        record_eventbus_stats(&registry, &stats).unwrap();

        assert_eq!(
            registry.gauge(NEBULA_EVENTBUS_SENT).unwrap().get(),
            i64::MAX
        );
        assert_eq!(
            registry.gauge(NEBULA_EVENTBUS_SUBSCRIBERS).unwrap().get(),
            i64::MAX
        );
        assert_eq!(
            registry
                .gauge(NEBULA_EVENTBUS_DROP_RATIO_PPM)
                .unwrap()
                .get(),
            0
        );
    }
}
