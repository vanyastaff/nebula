//! Label allowlist for preventing high-cardinality series in metric storage.
//!
//! When dynamic labels (e.g. action names, trigger types) are attached to
//! metrics, certain values — like `execution_id` or `workflow_id` — can cause
//! **cardinality explosion**: the registry grows by one entry per unique value,
//! and TSDB (Prometheus, VictoriaMetrics) accumulates unbounded time-series.
//!
//! A [`LabelAllowlist`] specifies which label keys are *safe* to include in
//! recorded metrics. Any key not on the list is stripped before the
//! [`LabelSet`] reaches the registry.
//!
//! ## Example
//!
//! ```rust
//! use std::sync::Arc;
//!
//! use nebula_metrics::{adapter::TelemetryAdapter, filter::LabelAllowlist};
//! use nebula_telemetry::metrics::MetricsRegistry;
//!
//! let reg = Arc::new(MetricsRegistry::new());
//! let adapter = TelemetryAdapter::new(Arc::clone(&reg));
//!
//! let allowlist = LabelAllowlist::only(["action_type", "status"]);
//!
//! // Safe: only low-cardinality keys pass.
//! let raw = reg.interner().label_set(&[
//!     ("action_type", "http.request"),
//!     ("execution_id", "550e8400-e29b-41d4-a716-446655440000"), // filtered out
//! ]);
//! let safe = allowlist.apply(&raw, reg.interner());
//! assert_eq!(safe.len(), 1);
//! ```

use nebula_telemetry::labels::{LabelInterner, LabelSet};

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
    Keys(Vec<String>),
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
    /// use nebula_metrics::filter::LabelAllowlist;
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
            inner: AllowlistInner::Keys(keys.into_iter().map(Into::into).collect()),
        }
    }

    /// Apply the allowlist to a [`LabelSet`], returning a filtered copy.
    ///
    /// If the allowlist is [`LabelAllowlist::all`], the original set is
    /// returned unchanged (cheap clone of the `Vec` of `Spur` pairs).
    ///
    /// Otherwise keys not present in the allowlist are stripped. Keys that
    /// are listed but not found in `labels` are silently ignored.
    #[must_use]
    pub fn apply(&self, labels: &LabelSet, interner: &LabelInterner) -> LabelSet {
        match &self.inner {
            AllowlistInner::All => labels.clone(),
            AllowlistInner::Keys(keys) => {
                let allowed: Vec<&str> = keys.iter().map(String::as_str).collect();
                interner.filter_label_set(labels, &allowed)
            }
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

    use nebula_telemetry::metrics::MetricsRegistry;

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
