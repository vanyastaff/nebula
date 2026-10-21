//! Label allowlist for preventing high-cardinality series in metric storage.
//!
//! When dynamic labels (e.g. action names, trigger types) are attached to
//! metrics, certain values — like `execution_id` or `workflow_id` — can cause
//! **cardinality explosion**: the registry grows by one entry per unique value,
//! and TSDB (Prometheus, VictoriaMetrics) accumulates unbounded time-series.
//!
//! Two guards live here:
//!
//! - [`CardinalityLimits`] is the policy every [`crate::MetricsRegistry`]
//!   enforces when it creates a series: it strips identifier and URL keys
//!   ([`HIGH_CARDINALITY_LABEL_KEYS`]) and caps the label sets per metric,
//!   folding the excess into an overflow series. It is on by default.
//! - A [`LabelAllowlist`] specifies which label keys are *safe* to include in
//!   recorded or exported metrics. Any key not on the list is stripped.
//!
//! ## Example
//!
//! ```rust
//! use nebula_metrics::{LabelAllowlist, MetricsRegistry};
//!
//! let reg = MetricsRegistry::new();
//! let allowlist = LabelAllowlist::only(["action_type", "status"]);
//!
//! // Only low-cardinality keys pass.
//! let raw = reg.interner().label_set(&[
//!     ("action_type", "http.request"),
//!     ("execution_id", "550e8400-e29b-41d4-a716-446655440000"), // filtered out
//! ]);
//! let safe = allowlist.apply(&raw, reg.interner());
//! assert_eq!(safe.len(), 1);
//! ```

use std::sync::{Arc, Mutex};

use crate::labels::{LabelInterner, LabelKey, LabelSet};

/// Default for [`CardinalityLimits::max_series_per_metric`].
pub const DEFAULT_MAX_SERIES_PER_METRIC: usize = 1_000;

/// Label value of the overflow series a capped metric folds new label sets into.
///
/// The overflow series keeps the label keys of the observation that overflowed
/// and sets every value to this string, so `sum by (key)` queries keep working.
/// A label set whose values are all this string is treated as an overflow
/// series and is always admitted.
pub const OVERFLOW_LABEL_VALUE: &str = "__overflow__";

/// Label keys that carry identifiers or URLs and are stripped by default.
///
/// Every value of such a key is a new series, so a counter labeled with one
/// grows by one series per execution, request, or user. Per-entity detail
/// belongs in tracing spans, not in metric labels.
pub const HIGH_CARDINALITY_LABEL_KEYS: &[&str] = &[
    "execution_id",
    "workflow_id",
    "node_id",
    "trigger_id",
    "correlation_id",
    "trace_id",
    "span_id",
    "request_id",
    "user_id",
    "session_id",
    "url",
    "uri",
];

/// Series-cardinality policy a [`crate::MetricsRegistry`] enforces when it
/// creates a series.
///
/// The default caps every metric at [`DEFAULT_MAX_SERIES_PER_METRIC`] label
/// sets and strips the keys in [`HIGH_CARDINALITY_LABEL_KEYS`]. Once a metric
/// has `max_series_per_metric` label sets, an observation with a new label set
/// is recorded on that metric's overflow series (see [`OVERFLOW_LABEL_VALUE`])
/// and counted in [`crate::NEBULA_METRICS_SERIES_OVERFLOW_TOTAL`]. The
/// unlabeled series and overflow series do not count toward the cap.
///
/// # Examples
///
/// ```rust
/// use nebula_metrics::{CardinalityLimits, MetricsRegistry};
///
/// let registry = MetricsRegistry::with_cardinality_limits(
///     CardinalityLimits::default().with_max_series_per_metric(50),
/// );
/// # let _ = registry;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardinalityLimits {
    max_series_per_metric: usize,
    denied_label_keys: Vec<String>,
}

impl CardinalityLimits {
    /// Set how many distinct label sets one metric name may hold.
    ///
    /// `0` folds every labeled observation into the overflow series.
    #[must_use]
    pub fn with_max_series_per_metric(mut self, max_series_per_metric: usize) -> Self {
        self.max_series_per_metric = max_series_per_metric;
        self
    }

    /// Replace the label keys the registry strips before creating a series.
    ///
    /// Pass an empty list to keep every key; the series cap still applies.
    #[must_use]
    pub fn with_denied_label_keys<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.denied_label_keys = keys.into_iter().map(Into::into).collect();
        self
    }

    /// Maximum number of distinct label sets per metric name.
    #[must_use]
    pub fn max_series_per_metric(&self) -> usize {
        self.max_series_per_metric
    }

    /// Label keys stripped before a series is created.
    #[must_use]
    pub fn denied_label_keys(&self) -> &[String] {
        &self.denied_label_keys
    }
}

impl Default for CardinalityLimits {
    fn default() -> Self {
        Self {
            max_series_per_metric: DEFAULT_MAX_SERIES_PER_METRIC,
            denied_label_keys: HIGH_CARDINALITY_LABEL_KEYS
                .iter()
                .map(|k| (*k).to_owned())
                .collect(),
        }
    }
}

/// A set of approved label-key names for metric observations.
///
/// Use [`LabelAllowlist::all`] to skip filtering entirely, or
/// [`LabelAllowlist::only`] to allow a specific set of low-cardinality keys.
///
/// Apply the filter with [`LabelAllowlist::apply`] before passing a
/// [`LabelSet`] to the registry.
#[derive(Debug, Clone)]
pub struct LabelAllowlist {
    inner: AllowlistInner,
}

#[derive(Debug, Clone)]
enum AllowlistInner {
    /// Pass every label through unchanged.
    All,
    /// Only allow keys whose names are in this list.
    Keys {
        key_strings: Arc<Vec<String>>,
        /// Lazily interned allow-list keys for the first registry interner seen.
        cached_spurs: Arc<Mutex<Option<Vec<LabelKey>>>>,
    },
}

impl LabelAllowlist {
    /// Allow **all** labels — no filtering applied.
    ///
    /// Use this as a no-op default when you do not need cardinality protection.
    #[must_use]
    pub fn all() -> Self {
        Self {
            inner: AllowlistInner::All,
        }
    }

    /// Allow only the specified label key names; all other keys are stripped.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_metrics::LabelAllowlist;
    ///
    /// let allow = LabelAllowlist::only(["action_type", "status", "trigger_type"]);
    /// ```
    #[must_use]
    pub fn only<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            inner: AllowlistInner::Keys {
                key_strings: Arc::new(keys.into_iter().map(Into::into).collect()),
                cached_spurs: Arc::new(Mutex::new(None)),
            },
        }
    }

    /// Apply the allowlist to a [`LabelSet`], returning a filtered copy.
    ///
    /// If the allowlist is [`LabelAllowlist::all`], the original set is
    /// returned unchanged (cheap clone of the `Vec` of `Spur` pairs).
    ///
    /// Otherwise keys not present in the allowlist are stripped. Keys that
    /// are listed but not found in `labels` are silently ignored.
    ///
    /// For [`LabelAllowlist::only`], allowed key names are interned once on the
    /// first call with a given `interner` and reused on subsequent `apply`
    /// calls to avoid re-hashing the allow-list on every observation.
    #[must_use]
    pub fn apply(&self, labels: &LabelSet, interner: &LabelInterner) -> LabelSet {
        match &self.inner {
            AllowlistInner::All => labels.clone(),
            AllowlistInner::Keys {
                key_strings,
                cached_spurs,
            } => {
                let mut guard = cached_spurs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let allowed = guard.get_or_insert_with(|| {
                    key_strings
                        .iter()
                        .map(|k| interner.intern(k.as_str()))
                        .collect()
                });
                interner.filter_label_set_by_spur(labels, allowed)
            },
        }
    }

    /// Returns `true` if this allowlist passes all labels through (i.e. was
    /// created with [`LabelAllowlist::all`]).
    #[must_use]
    pub fn is_passthrough(&self) -> bool {
        matches!(self.inner, AllowlistInner::All)
    }
}

impl Default for LabelAllowlist {
    fn default() -> Self {
        Self::all()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::registry::MetricsRegistry;

    use super::*;

    fn registry() -> Arc<MetricsRegistry> {
        Arc::new(MetricsRegistry::new())
    }

    #[test]
    fn all_passes_every_label() {
        let reg = registry();
        let labels = reg
            .interner()
            .label_set(&[("action_type", "http.request"), ("execution_id", "uuid-1")]);
        let filtered = LabelAllowlist::all().apply(&labels, reg.interner());
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn only_strips_unlisted_keys() {
        let reg = registry();
        let labels = reg.interner().label_set(&[
            ("action_type", "http.request"),
            ("execution_id", "uuid-abc"),
            ("status", "success"),
        ]);
        let allow = LabelAllowlist::only(["action_type", "status"]);
        let filtered = allow.apply(&labels, reg.interner());
        assert_eq!(filtered.len(), 2);

        let pairs = filtered.resolve(reg.interner());
        let keys: Vec<&str> = pairs.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"action_type"));
        assert!(keys.contains(&"status"));
        assert!(!keys.contains(&"execution_id"));
    }

    #[test]
    fn only_with_no_matching_keys_returns_empty() {
        let reg = registry();
        let labels = reg
            .interner()
            .label_set(&[("execution_id", "uuid-1"), ("workflow_id", "uuid-2")]);
        let allow = LabelAllowlist::only(["action_type"]);
        let filtered = allow.apply(&labels, reg.interner());
        assert_eq!(filtered.len(), 0);
    }

    #[test]
    fn is_passthrough_for_all() {
        assert!(LabelAllowlist::all().is_passthrough());
        assert!(!LabelAllowlist::only(["k"]).is_passthrough());
    }

    #[test]
    fn default_is_passthrough() {
        assert!(LabelAllowlist::default().is_passthrough());
    }
}
