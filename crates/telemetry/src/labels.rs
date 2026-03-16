//! Label support for metric observations.
//!
//! Labels (also known as dimensions or tags) let you attach key-value metadata
//! to metric observations — e.g. `action_type = "http.request"` or
//! `status = "success"`.
//!
//! ## Design
//!
//! All label keys **and** values are interned via [`lasso::ThreadedRodeo`].
//! Repeated strings — action names, status codes, resource keys — cost a
//! single allocation on first production and only an integer equality check
//! thereafter. A [`LabelSet`] is then a compact, hash-safe `Vec<(Spur,
//! Spur)>` sorted by key.
//!
//! ## Usage
//!
//! ```rust
//! use nebula_telemetry::labels::{LabelInterner, LabelSet};
//!
//! let interner = LabelInterner::new();
//! let labels = interner.label_set(&[("action_type", "http.request"), ("status", "success")]);
//! assert_eq!(labels.len(), 2);
//! ```

use std::sync::Arc;

use lasso::{Spur, ThreadedRodeo};

// ── LabelKey / LabelValue ────────────────────────────────────────────────────

/// An interned label key handle.
///
/// Equivalent to a `&'static str` identity but heap-allocated once and then
/// compared by integer value (`Spur` is a `u32`-backed index).
pub type LabelKey = Spur;

/// An interned label value handle.
pub type LabelValue = Spur;

// ── LabelSet ─────────────────────────────────────────────────────────────────

/// An ordered, interned set of metric label key-value pairs.
///
/// Pairs are sorted by key `Spur` on construction so that two sets built
/// from the same entries in different orders compare equal and hash to the
/// same bucket.
///
/// # Examples
///
/// ```rust
/// use nebula_telemetry::labels::{LabelInterner, LabelSet};
///
/// let interner = LabelInterner::new();
/// let a = interner.label_set(&[("status", "ok"), ("action", "http.request")]);
/// let b = interner.label_set(&[("action", "http.request"), ("status", "ok")]);
/// assert_eq!(a, b);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct LabelSet {
    pairs: Vec<(LabelKey, LabelValue)>,
}

impl LabelSet {
    /// Create an empty label set (metric with no labels).
    #[must_use]
    pub fn empty() -> Self {
        Self { pairs: Vec::new() }
    }

    /// Number of labels in this set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// Whether the set contains no labels.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Iterate over the raw `(LabelKey, LabelValue)` spur pairs.
    pub fn iter(&self) -> impl Iterator<Item = (LabelKey, LabelValue)> + '_ {
        self.pairs.iter().copied()
    }

    /// Resolve all spurs back to `&str` slices using the provided interner.
    ///
    /// Returns `(key, value)` string pairs sorted by key.
    pub fn resolve<'a>(&'a self, interner: &'a LabelInterner) -> Vec<(&'a str, &'a str)> {
        self.pairs
            .iter()
            .map(|(k, v)| (interner.resolve(*k), interner.resolve(*v)))
            .collect()
    }
}

// ── LabelInterner ─────────────────────────────────────────────────────────────

/// Thread-safe string interner for label keys and values.
///
/// Backed by [`lasso::ThreadedRodeo`].  Cheaply cloneable via `Arc`.
/// All interning operations are lock-free for reads; first-time registrations  
/// acquire an internal write lock.
///
/// # Examples
///
/// ```rust
/// use nebula_telemetry::labels::LabelInterner;
///
/// let interner = LabelInterner::new();
/// let spur = interner.intern("http.request");
/// assert_eq!(interner.resolve(spur), "http.request");
///
/// // Same string → same spur.
/// let spur2 = interner.intern("http.request");
/// assert_eq!(spur, spur2);
/// ```
#[derive(Clone, Debug)]
pub struct LabelInterner {
    rodeo: Arc<ThreadedRodeo>,
}

impl LabelInterner {
    /// Create a fresh interner.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rodeo: Arc::new(ThreadedRodeo::new()),
        }
    }

    /// Intern a string and return its stable [`Spur`] handle.
    ///
    /// If the string is already interned the existing handle is returned
    /// without any allocation.
    pub fn intern(&self, s: &str) -> Spur {
        self.rodeo.get_or_intern(s)
    }

    /// Resolve a [`Spur`] back to its original string slice.
    ///
    /// # Panics
    ///
    /// Panics if the spur was not produced by this interner (different instance).
    #[must_use]
    pub fn resolve(&self, spur: Spur) -> &str {
        self.rodeo.resolve(&spur)
    }

    /// Try to resolve a [`Spur`] without panicking.
    #[must_use]
    pub fn try_resolve(&self, spur: Spur) -> Option<&str> {
        self.rodeo.try_resolve(&spur)
    }

    /// Build a [`LabelSet`] from string key-value pairs.
    ///
    /// All keys and values are interned; the resulting set is sorted by key.
    #[must_use]
    pub fn label_set(&self, pairs: &[(&str, &str)]) -> LabelSet {
        let mut kv: Vec<(LabelKey, LabelValue)> = pairs
            .iter()
            .map(|(k, v)| (self.intern(k), self.intern(v)))
            .collect();
        // Sort by key Spur so equal label sets compare and hash identically
        // regardless of insertion order.
        kv.sort_unstable_by_key(|(k, _)| *k);
        LabelSet { pairs: kv }
    }

    /// Build a [`LabelSet`] from a single key-value pair.
    #[must_use]
    pub fn single(&self, key: &str, value: &str) -> LabelSet {
        let k = self.intern(key);
        let v = self.intern(value);
        LabelSet {
            pairs: vec![(k, v)],
        }
    }

    /// Return a new [`LabelSet`] containing only pairs whose key is in `allowed_keys`.
    ///
    /// Keys not yet interned (i.e. not present in the interner) can never
    /// appear in an existing `LabelSet`, so they are silently ignored.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_telemetry::labels::LabelInterner;
    ///
    /// let interner = LabelInterner::new();
    /// let labels = interner.label_set(&[
    ///     ("action_type", "http.request"),
    ///     ("execution_id", "uuid-abc"),
    /// ]);
    /// let safe = interner.filter_label_set(&labels, &["action_type"]);
    /// assert_eq!(safe.len(), 1);
    /// ```
    #[must_use]
    pub fn filter_label_set(&self, labels: &LabelSet, allowed_keys: &[&str]) -> LabelSet {
        // Intern the allowed keys so that we compare Spur ↔ Spur (integers).
        let allowed: Vec<Spur> = allowed_keys.iter().map(|k| self.intern(k)).collect();
        let pairs: Vec<(LabelKey, LabelValue)> = labels
            .iter()
            .filter(|(k, _)| allowed.contains(k))
            .collect();
        // Already sorted because the source LabelSet is sorted.
        LabelSet { pairs }
    }
}

impl Default for LabelInterner {
    fn default() -> Self {
        Self::new()
    }
}

// ── A composite registry key: metric name + sorted labels ────────────────────

/// Composite key used internally by [`MetricsRegistry`] for labeled metrics.
///
/// Combines the interned metric name with a [`LabelSet`] for efficient
/// per-series lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetricKey {
    /// Interned metric name.
    pub name: Spur,
    /// Label set (may be empty for unlabeled metrics).
    pub labels: LabelSet,
}

impl MetricKey {
    /// Create a key for an unlabeled metric.
    #[must_use]
    pub fn unlabeled(name: Spur) -> Self {
        Self {
            name,
            labels: LabelSet::empty(),
        }
    }

    /// Create a key for a labeled metric.
    #[must_use]
    pub fn labeled(name: Spur, labels: LabelSet) -> Self {
        Self { name, labels }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_set_order_invariant() {
        let interner = LabelInterner::new();
        let a = interner.label_set(&[("status", "ok"), ("action", "http.request")]);
        let b = interner.label_set(&[("action", "http.request"), ("status", "ok")]);
        assert_eq!(a, b, "LabelSet must be order-invariant");
    }

    #[test]
    fn intern_same_string_returns_same_spur() {
        let interner = LabelInterner::new();
        let s1 = interner.intern("nebula_executions_total");
        let s2 = interner.intern("nebula_executions_total");
        assert_eq!(s1, s2);
    }

    #[test]
    fn intern_different_strings_returns_different_spurs() {
        let interner = LabelInterner::new();
        let s1 = interner.intern("counter_a");
        let s2 = interner.intern("counter_b");
        assert_ne!(s1, s2);
    }

    #[test]
    fn resolve_roundtrip() {
        let interner = LabelInterner::new();
        let spur = interner.intern("nebula_action_duration_seconds");
        assert_eq!(interner.resolve(spur), "nebula_action_duration_seconds");
    }

    #[test]
    fn empty_label_set_is_default() {
        assert!(LabelSet::empty().is_empty());
        assert_eq!(LabelSet::empty(), LabelSet::default());
    }

    #[test]
    fn label_set_len() {
        let interner = LabelInterner::new();
        let ls = interner.label_set(&[("k1", "v1"), ("k2", "v2"), ("k3", "v3")]);
        assert_eq!(ls.len(), 3);
    }

    #[test]
    fn metric_key_equality() {
        let interner = LabelInterner::new();
        let name = interner.intern("counter");
        let ls = interner.label_set(&[("env", "prod")]);
        let k1 = MetricKey::labeled(name, ls.clone());
        let k2 = MetricKey::labeled(name, ls);
        assert_eq!(k1, k2);
    }
}
