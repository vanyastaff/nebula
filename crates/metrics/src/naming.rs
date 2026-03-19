//! Standard metric names for Nebula.
//!
//! Convention: `nebula_<domain>_<metric>_<unit>`.
//! See [docs/crates/metrics/TARGET.md](https://github.com/vanyastaff/nebula/blob/main/docs/crates/metrics/TARGET.md).

use std::fmt;

/// The type of metric primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    /// Monotonically increasing counter.
    Counter,
    /// Point-in-time gauge.
    Gauge,
    /// Distribution of observed values.
    Histogram,
}

impl fmt::Display for MetricKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Counter => f.write_str("counter"),
            Self::Gauge => f.write_str("gauge"),
            Self::Histogram => f.write_str("histogram"),
        }
    }
}

/// A well-known Nebula metric with its name, kind, and help text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MetricName {
    name: &'static str,
    kind: MetricKind,
    help: &'static str,
}

impl MetricName {
    /// Returns the Prometheus-format metric name string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.name
    }

    /// Returns the kind of metric (counter, gauge, histogram).
    #[must_use]
    pub const fn kind(&self) -> MetricKind {
        self.kind
    }

    /// Returns the human-readable help/description text.
    #[must_use]
    pub const fn help(&self) -> &'static str {
        self.help
    }
}

impl AsRef<str> for MetricName {
    fn as_ref(&self) -> &str {
        self.name
    }
}

impl fmt::Display for MetricName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

// ---------------------------------------------------------------------------
// Workflow (engine)
// ---------------------------------------------------------------------------

/// Counter: workflow executions started.
pub const WORKFLOW_EXECUTIONS_STARTED: MetricName = MetricName {
    name: "nebula_workflow_executions_started_total",
    kind: MetricKind::Counter,
    help: "Total workflow executions started.",
};

/// Counter: workflow executions completed successfully.
pub const WORKFLOW_EXECUTIONS_COMPLETED: MetricName = MetricName {
    name: "nebula_workflow_executions_completed_total",
    kind: MetricKind::Counter,
    help: "Total workflow executions completed successfully.",
};

/// Counter: workflow executions failed.
pub const WORKFLOW_EXECUTIONS_FAILED: MetricName = MetricName {
    name: "nebula_workflow_executions_failed_total",
    kind: MetricKind::Counter,
    help: "Total workflow executions failed.",
};

/// Histogram: workflow execution duration in seconds.
pub const WORKFLOW_EXECUTION_DURATION: MetricName = MetricName {
    name: "nebula_workflow_execution_duration_seconds",
    kind: MetricKind::Histogram,
    help: "Workflow execution duration in seconds.",
};

// ---------------------------------------------------------------------------
// Action (runtime)
// ---------------------------------------------------------------------------

/// Counter: action executions (success + failure).
pub const ACTION_EXECUTIONS: MetricName = MetricName {
    name: "nebula_action_executions_total",
    kind: MetricKind::Counter,
    help: "Total action executions.",
};

/// Counter: action failures.
pub const ACTION_FAILURES: MetricName = MetricName {
    name: "nebula_action_failures_total",
    kind: MetricKind::Counter,
    help: "Total action failures.",
};

/// Histogram: action execution duration in seconds.
pub const ACTION_DURATION: MetricName = MetricName {
    name: "nebula_action_duration_seconds",
    kind: MetricKind::Histogram,
    help: "Action execution duration in seconds.",
};

// ---------------------------------------------------------------------------
// Resource (resource crate)
// ---------------------------------------------------------------------------

/// Counter: resource instances created.
pub const RESOURCE_CREATE: MetricName = MetricName {
    name: "nebula_resource_create_total",
    kind: MetricKind::Counter,
    help: "Total resource instances created.",
};

/// Counter: resource acquisitions.
pub const RESOURCE_ACQUIRE: MetricName = MetricName {
    name: "nebula_resource_acquire_total",
    kind: MetricKind::Counter,
    help: "Total resource acquisitions.",
};

/// Histogram: wait time before acquisition in seconds.
pub const RESOURCE_ACQUIRE_WAIT_DURATION: MetricName = MetricName {
    name: "nebula_resource_acquire_wait_duration_seconds",
    kind: MetricKind::Histogram,
    help: "Wait time before resource acquisition in seconds.",
};

/// Counter: resource releases.
pub const RESOURCE_RELEASE: MetricName = MetricName {
    name: "nebula_resource_release_total",
    kind: MetricKind::Counter,
    help: "Total resource releases.",
};

/// Histogram: usage duration in seconds.
pub const RESOURCE_USAGE_DURATION: MetricName = MetricName {
    name: "nebula_resource_usage_duration_seconds",
    kind: MetricKind::Histogram,
    help: "Resource usage duration in seconds.",
};

/// Counter: resource cleanups.
pub const RESOURCE_CLEANUP: MetricName = MetricName {
    name: "nebula_resource_cleanup_total",
    kind: MetricKind::Counter,
    help: "Total resource cleanups.",
};

/// Counter: resource errors.
pub const RESOURCE_ERROR: MetricName = MetricName {
    name: "nebula_resource_error_total",
    kind: MetricKind::Counter,
    help: "Total resource errors.",
};

/// Gauge: health state (1=healthy, 0.5=degraded/unknown, 0=unhealthy).
pub const RESOURCE_HEALTH_STATE: MetricName = MetricName {
    name: "nebula_resource_health_state",
    kind: MetricKind::Gauge,
    help: "Resource health state (1=healthy, 0.5=degraded, 0=unhealthy).",
};

/// Counter: pool exhausted events.
pub const RESOURCE_POOL_EXHAUSTED: MetricName = MetricName {
    name: "nebula_resource_pool_exhausted_total",
    kind: MetricKind::Counter,
    help: "Total pool exhaustion events.",
};

/// Gauge: number of waiters when pool exhausted.
pub const RESOURCE_POOL_WAITERS: MetricName = MetricName {
    name: "nebula_resource_pool_waiters",
    kind: MetricKind::Gauge,
    help: "Number of waiters when pool exhausted.",
};

/// Counter: resources quarantined.
pub const RESOURCE_QUARANTINE: MetricName = MetricName {
    name: "nebula_resource_quarantine_total",
    kind: MetricKind::Counter,
    help: "Total resources quarantined.",
};

/// Counter: resources released from quarantine.
pub const RESOURCE_QUARANTINE_RELEASED: MetricName = MetricName {
    name: "nebula_resource_quarantine_released_total",
    kind: MetricKind::Counter,
    help: "Total resources released from quarantine.",
};

/// Counter: config reloads.
pub const RESOURCE_CONFIG_RELOADED: MetricName = MetricName {
    name: "nebula_resource_config_reloaded_total",
    kind: MetricKind::Counter,
    help: "Total config reloads.",
};

/// Counter: credential rotations applied to a resource pool.
pub const RESOURCE_CREDENTIAL_ROTATED: MetricName = MetricName {
    name: "nebula_resource_credential_rotated_total",
    kind: MetricKind::Counter,
    help: "Total credential rotations applied.",
};

/// Counter: circuit breaker transitioned to open state.
pub const RESOURCE_CIRCUIT_BREAKER_OPENED: MetricName = MetricName {
    name: "nebula_resource_circuit_breaker_opened_total",
    kind: MetricKind::Counter,
    help: "Total circuit breaker open transitions.",
};

/// Counter: circuit breaker transitioned to closed state (recovered).
pub const RESOURCE_CIRCUIT_BREAKER_CLOSED: MetricName = MetricName {
    name: "nebula_resource_circuit_breaker_closed_total",
    kind: MetricKind::Counter,
    help: "Total circuit breaker close transitions.",
};

// ---------------------------------------------------------------------------
// EventBus (generic bus layer)
// ---------------------------------------------------------------------------

/// Gauge: snapshot of sent events for an EventBus instance.
pub const EVENTBUS_SENT: MetricName = MetricName {
    name: "nebula_eventbus_sent",
    kind: MetricKind::Gauge,
    help: "EventBus sent events snapshot.",
};

/// Gauge: snapshot of dropped events for an EventBus instance.
pub const EVENTBUS_DROPPED: MetricName = MetricName {
    name: "nebula_eventbus_dropped",
    kind: MetricKind::Gauge,
    help: "EventBus dropped events snapshot.",
};

/// Gauge: snapshot of active subscribers for an EventBus instance.
pub const EVENTBUS_SUBSCRIBERS: MetricName = MetricName {
    name: "nebula_eventbus_subscribers",
    kind: MetricKind::Gauge,
    help: "EventBus active subscribers snapshot.",
};

/// Gauge: snapshot drop ratio (`0.0..=1.0`) scaled by 1_000_000.
pub const EVENTBUS_DROP_RATIO_PPM: MetricName = MetricName {
    name: "nebula_eventbus_drop_ratio_ppm",
    kind: MetricKind::Gauge,
    help: "EventBus drop ratio in parts-per-million.",
};

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

/// All well-known metric definitions in the Nebula workspace.
pub const ALL_METRICS: &[MetricName] = &[
    // Workflow
    WORKFLOW_EXECUTIONS_STARTED,
    WORKFLOW_EXECUTIONS_COMPLETED,
    WORKFLOW_EXECUTIONS_FAILED,
    WORKFLOW_EXECUTION_DURATION,
    // Action
    ACTION_EXECUTIONS,
    ACTION_FAILURES,
    ACTION_DURATION,
    // Resource
    RESOURCE_CREATE,
    RESOURCE_ACQUIRE,
    RESOURCE_ACQUIRE_WAIT_DURATION,
    RESOURCE_RELEASE,
    RESOURCE_USAGE_DURATION,
    RESOURCE_CLEANUP,
    RESOURCE_ERROR,
    RESOURCE_HEALTH_STATE,
    RESOURCE_POOL_EXHAUSTED,
    RESOURCE_POOL_WAITERS,
    RESOURCE_QUARANTINE,
    RESOURCE_QUARANTINE_RELEASED,
    RESOURCE_CONFIG_RELOADED,
    RESOURCE_CREDENTIAL_ROTATED,
    RESOURCE_CIRCUIT_BREAKER_OPENED,
    RESOURCE_CIRCUIT_BREAKER_CLOSED,
    // EventBus
    EVENTBUS_SENT,
    EVENTBUS_DROPPED,
    EVENTBUS_SUBSCRIBERS,
    EVENTBUS_DROP_RATIO_PPM,
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nebula_telemetry::metrics::MetricsRegistry;

    use super::*;

    const RESOURCE_METRICS: [MetricName; 14] = [
        RESOURCE_CREATE,
        RESOURCE_ACQUIRE,
        RESOURCE_ACQUIRE_WAIT_DURATION,
        RESOURCE_RELEASE,
        RESOURCE_USAGE_DURATION,
        RESOURCE_CLEANUP,
        RESOURCE_ERROR,
        RESOURCE_HEALTH_STATE,
        RESOURCE_POOL_EXHAUSTED,
        RESOURCE_POOL_WAITERS,
        RESOURCE_QUARANTINE,
        RESOURCE_QUARANTINE_RELEASED,
        RESOURCE_CONFIG_RELOADED,
        RESOURCE_CREDENTIAL_ROTATED,
    ];

    fn assert_naming_convention(names: &[&str], prefix: &str) {
        let mut unique = HashSet::new();
        for name in names {
            assert!(!name.is_empty(), "constant must not be empty");
            assert!(name.starts_with(prefix), "{name} must start with {prefix}");
            assert!(
                name.chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
                "{name} contains invalid chars"
            );
            assert!(unique.insert(*name), "{name} is duplicated");
        }
    }

    #[test]
    fn workflow_constants_follow_naming_convention() {
        assert_naming_convention(
            &[
                WORKFLOW_EXECUTIONS_STARTED.as_str(),
                WORKFLOW_EXECUTIONS_COMPLETED.as_str(),
                WORKFLOW_EXECUTIONS_FAILED.as_str(),
                WORKFLOW_EXECUTION_DURATION.as_str(),
            ],
            "nebula_workflow_",
        );
    }

    #[test]
    fn action_constants_follow_naming_convention() {
        assert_naming_convention(
            &[
                ACTION_EXECUTIONS.as_str(),
                ACTION_FAILURES.as_str(),
                ACTION_DURATION.as_str(),
            ],
            "nebula_action_",
        );
    }

    #[test]
    fn eventbus_constants_follow_naming_convention() {
        assert_naming_convention(
            &[
                EVENTBUS_SENT.as_str(),
                EVENTBUS_DROPPED.as_str(),
                EVENTBUS_SUBSCRIBERS.as_str(),
                EVENTBUS_DROP_RATIO_PPM.as_str(),
            ],
            "nebula_eventbus_",
        );
    }

    #[test]
    fn resource_constants_are_accessible_unique_and_registry_safe() {
        let registry = MetricsRegistry::new();
        let mut unique = HashSet::new();

        for metric in RESOURCE_METRICS {
            let metric_name = metric.as_str();
            tracing::debug!("testing constant: {}", metric_name);
            assert!(!metric_name.is_empty());
            assert!(metric_name.starts_with("nebula_resource_"));
            assert!(
                metric_name
                    .chars()
                    .all(|ch| { ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' })
            );
            assert!(unique.insert(metric_name));

            let counter = registry.counter(metric_name);
            counter.inc();
            assert_eq!(counter.get(), 1);
        }

        assert_eq!(unique.len(), 14);
    }

    #[test]
    fn all_metrics_contains_all_26_entries() {
        assert_eq!(ALL_METRICS.len(), 27);
        let unique: HashSet<&str> = ALL_METRICS.iter().map(|m| m.as_str()).collect();
        assert_eq!(unique.len(), 27, "all metric names must be unique");
    }

    #[test]
    fn metric_name_display_matches_as_str() {
        for metric in ALL_METRICS {
            assert_eq!(metric.to_string(), metric.as_str());
        }
    }

    #[test]
    fn metric_name_as_ref_matches_as_str() {
        for metric in ALL_METRICS {
            let r: &str = metric.as_ref();
            assert_eq!(r, metric.as_str());
        }
    }
}
