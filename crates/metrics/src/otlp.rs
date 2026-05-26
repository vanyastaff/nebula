//! OTLP metrics exporter on top of the `MetricsRegistry` snapshot seam.
//!
//! Pushes the in-memory metric snapshots produced by
//! [`crate::registry::MetricsRegistry::snapshot_counters`],
//! [`crate::registry::MetricsRegistry::snapshot_gauges`], and
//! [`crate::registry::MetricsRegistry::snapshot_histograms`] to an OTLP collector via the OTel
//! SDK's [`opentelemetry_sdk::metrics::SdkMeterProvider`] + [`opentelemetry_otlp::MetricExporter`]
//! pipeline.
//!
//! ## Architecture (ADR-0046)
//!
//! Per ADR-0046 ("metrics/telemetry boundary") this module is the single seam between
//! `nebula-metrics` and the OpenTelemetry SDK. All OTel SDK calls live here; the rest of the
//! crate (`MetricsRegistry`, primitives, naming, label allowlist) is unchanged. The exporter
//! registers OTel observable instruments whose callbacks re-read the registry on every OTel
//! collection cycle, so the registry's snapshot contract is the public contract.
//!
//! ## Discovery
//!
//! Nebula's metric registry is populated lazily — counters/gauges/histograms come into
//! existence the first time the producing code path runs. To pick those up without forcing a
//! restart, [`OtlpMetricsExporter`] spawns a background discovery task that wakes at half the
//! configured export interval, snapshots the registry, and registers a new OTel observable
//! instrument for any `(metric name, kind)` pair it has not seen before. Once registered, the
//! instrument's callback re-enumerates the snapshot on every OTel collection cycle and emits
//! one observation per label combination.
//!
//! ## Histograms
//!
//! OpenTelemetry does not define an observable histogram. Nebula's histograms are already
//! aggregated (bucket counts + sum + total count) at snapshot time, so this module emits each
//! histogram as three companion instruments matching the Prometheus convention:
//!
//! - `<name>_sum`    → `f64_observable_counter` (cumulative sum)
//! - `<name>_count`  → `u64_observable_counter` (cumulative observation count)
//! - `<name>_bucket` → `u64_observable_counter` with an `le` attribute (cumulative bucket
//!   counts; each `le` value is monotonic for a fixed label combination, so OTel counter
//!   semantics hold)
//!
//! Backends that consume the OTLP / Prometheus convention reconstruct the histogram from these
//! three series automatically.
//!
//! ## Cardinality
//!
//! All emitted labels pass through the configured [`LabelAllowlist`] (default:
//! [`LabelAllowlist::all`], unchanged behaviour). Operators can pass [`LabelAllowlist::only`]
//! to strip high-cardinality keys before they reach the collector.

use std::{
    borrow::Cow,
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use opentelemetry::{KeyValue, metrics::MeterProvider as _};
use opentelemetry_otlp::{ExporterBuildError, MetricExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    metrics::{PeriodicReader, SdkMeterProvider, Temporality, exporter::PushMetricExporter},
};

use crate::{filter::LabelAllowlist, labels::LabelSet, registry::MetricsRegistry};

/// Default `service.name` resource attribute when the caller does not provide one.
const DEFAULT_SERVICE_NAME: &str = "nebula-metrics";

/// Default export interval matching the OTel SDK default (60s) but explicit so the discovery
/// task and the periodic reader share the same value.
const DEFAULT_EXPORT_INTERVAL: Duration = Duration::from_mins(1);

/// Tracer-style instrumentation name on the OTel `Meter`; identifies *this library* as the
/// producer, distinct from `service.name` (the deploying process).
const METER_INSTRUMENTATION_NAME: &str = "nebula-metrics";

/// Configuration for [`OtlpMetricsExporter::install`].
///
/// The endpoint is required (callers typically read `OTEL_EXPORTER_OTLP_ENDPOINT` and skip
/// installation when it is unset / opt-out). All other fields default to the OTel-recommended
/// values: cumulative temporality, 60-second export interval, and a pass-through label
/// allowlist (no cardinality stripping).
#[derive(Debug, Clone)]
pub struct OtlpMetricsConfig {
    /// gRPC endpoint of the OTLP collector (e.g. `http://localhost:4317`).
    pub endpoint: String,
    /// `service.name` resource attribute reported on every metric export.
    pub service_name: String,
    /// How often the OTel `PeriodicReader` runs an export cycle.
    pub export_interval: Duration,
    /// Cardinality guard applied to every observed label set (default: pass-through).
    pub allowlist: LabelAllowlist,
    /// Output temporality (default: cumulative — the canonical OTel default).
    pub temporality: Temporality,
}

impl OtlpMetricsConfig {
    /// Build a config with the given endpoint and all other fields defaulted.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            service_name: DEFAULT_SERVICE_NAME.to_owned(),
            export_interval: DEFAULT_EXPORT_INTERVAL,
            allowlist: LabelAllowlist::all(),
            temporality: Temporality::Cumulative,
        }
    }

    /// Override the `service.name` resource attribute.
    #[must_use]
    pub fn with_service_name(mut self, name: impl Into<String>) -> Self {
        self.service_name = name.into();
        self
    }

    /// Override the export interval (the OTel `PeriodicReader` cadence).
    #[must_use]
    pub fn with_export_interval(mut self, interval: Duration) -> Self {
        self.export_interval = interval;
        self
    }

    /// Override the label cardinality guard. Use [`LabelAllowlist::only`] to strip
    /// high-cardinality keys before they reach the collector.
    #[must_use]
    pub fn with_allowlist(mut self, allowlist: LabelAllowlist) -> Self {
        self.allowlist = allowlist;
        self
    }

    /// Override the OTel temporality (default: cumulative).
    #[must_use]
    pub fn with_temporality(mut self, temporality: Temporality) -> Self {
        self.temporality = temporality;
        self
    }
}

/// Errors raised by [`OtlpMetricsExporter::install`].
#[derive(Debug, thiserror::Error)]
pub enum OtlpInitError {
    /// The OTLP `MetricExporter` failed to build (invalid endpoint, missing tonic runtime,
    /// etc.).
    #[error("OTLP metric exporter build failed: {0}")]
    ExporterBuild(#[from] ExporterBuildError),
}

/// Guard returned by [`OtlpMetricsExporter::install`].
///
/// The guard owns the OTel `SdkMeterProvider` and stops the background discovery task on
/// `Drop`. Holding the guard for the lifetime of the binary is required so the periodic reader
/// keeps flushing snapshots; dropping it triggers a deterministic provider shutdown which
/// flushes any in-flight export.
#[must_use = "Drop the guard at process shutdown so OTLP exports are flushed and the discovery task can stop"]
pub struct OtlpMetricsGuard {
    provider: Option<SdkMeterProvider>,
    stop: Arc<AtomicBool>,
}

impl OtlpMetricsGuard {
    /// Explicit shutdown — flushes the periodic reader and stops the discovery task.
    ///
    /// Called automatically on `Drop`; exposed so callers that want a deterministic shutdown
    /// (e.g. after `axum::serve` returns) can drain exports before process exit.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(provider) = self.provider.take()
            && let Err(err) = provider.shutdown()
        {
            // tracing may have already torn down its dispatch on process exit, so route
            // through eprintln! the same way the trace-side exporter does.
            eprintln!("nebula_metrics::otlp: meter provider shutdown error: {err}");
        }
    }
}

impl Drop for OtlpMetricsGuard {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Static handle holding the bookkeeping shared by every observable instrument callback.
struct ExporterInner {
    registry: Arc<MetricsRegistry>,
    allowlist: LabelAllowlist,
    /// `(name, instrument-role)` pairs already registered so the discovery task doesn't
    /// double-register. For counters/gauges the role is the bare metric name; for histograms
    /// the role disambiguates between `_sum`, `_count`, and `_bucket`.
    seen: Mutex<HashSet<(String, &'static str)>>,
}

impl ExporterInner {
    fn new(registry: Arc<MetricsRegistry>, allowlist: LabelAllowlist) -> Self {
        Self {
            registry,
            allowlist,
            seen: Mutex::new(HashSet::new()),
        }
    }

    /// Resolve `labels` through the registry interner, apply the allowlist, and return the
    /// resulting OTel `KeyValue` slice plus any extra (already-typed) attributes appended.
    fn build_attributes(&self, labels: &LabelSet, extras: &[KeyValue]) -> Vec<KeyValue> {
        let interner = self.registry.interner();
        let filtered = self.allowlist.apply(labels, interner);
        let resolved = filtered.resolve(interner);
        let mut out = Vec::with_capacity(resolved.len() + extras.len());
        for (k, v) in resolved {
            out.push(KeyValue::new(k.to_owned(), v.to_owned()));
        }
        out.extend_from_slice(extras);
        out
    }
}

/// Public façade installing the OTLP metrics pipeline against a [`MetricsRegistry`].
///
/// The struct itself holds no mutable state once `install` returns; lifetime ownership lives
/// in [`OtlpMetricsGuard`].
pub struct OtlpMetricsExporter;

impl OtlpMetricsExporter {
    /// Build an OTLP push pipeline targeting `config.endpoint` and bind it to `registry`.
    ///
    /// Spawns a background tokio task that discovers new metric names in the registry and
    /// registers OTel observable instruments for them. Requires a running tokio runtime
    /// (`#[tokio::main]` or `tokio::runtime::Runtime::block_on`).
    ///
    /// # Errors
    ///
    /// Returns [`OtlpInitError::ExporterBuild`] when the OTLP exporter cannot be constructed
    /// (e.g. malformed endpoint or missing tonic runtime).
    pub fn install(
        registry: Arc<MetricsRegistry>,
        config: OtlpMetricsConfig,
    ) -> Result<OtlpMetricsGuard, OtlpInitError> {
        let exporter = MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&config.endpoint)
            .with_temporality(config.temporality)
            .build()?;
        Ok(Self::install_with_exporter(registry, exporter, &config))
    }

    /// Variant of [`Self::install`] that accepts a pre-built [`PushMetricExporter`].
    ///
    /// Production code wires the OTLP gRPC exporter via [`Self::install`]; tests use this
    /// entry point with an in-memory exporter
    /// (`opentelemetry_sdk::metrics::InMemoryMetricExporter`) to assert the registry → OTel
    /// pipeline contract without spinning up a real collector.
    ///
    /// The exporter's temporality is decided by the caller (via the concrete exporter type);
    /// only `config.export_interval`, `config.service_name`, and `config.allowlist` are read
    /// here. The `config.endpoint` field is ignored.
    pub fn install_with_exporter<E>(
        registry: Arc<MetricsRegistry>,
        exporter: E,
        config: &OtlpMetricsConfig,
    ) -> OtlpMetricsGuard
    where
        E: PushMetricExporter,
    {
        let reader = PeriodicReader::builder(exporter)
            .with_interval(config.export_interval)
            .build();

        let resource = Resource::builder_empty()
            .with_attributes([KeyValue::new("service.name", config.service_name.clone())])
            .build();

        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource)
            .build();

        let inner = Arc::new(ExporterInner::new(
            Arc::clone(&registry),
            config.allowlist.clone(),
        ));
        let meter = provider.meter(METER_INSTRUMENTATION_NAME);

        // Run discovery once synchronously so the common case of "registry already
        // populated at install time" reports values on the very first export cycle.
        discover_and_register(&meter, &inner);

        let stop = Arc::new(AtomicBool::new(false));
        spawn_discovery_task(
            meter,
            Arc::clone(&inner),
            config.export_interval,
            Arc::clone(&stop),
        );

        OtlpMetricsGuard {
            provider: Some(provider),
            stop,
        }
    }
}

/// Spawn the background discovery task. Wakes at half the export interval (with a floor) and
/// re-runs [`discover_and_register`]. Stops cleanly when the guard sets `stop`.
///
/// Fails closed when no Tokio runtime is available: instead of letting
/// `tokio::spawn` panic on the library path, the function logs a warning and
/// returns without spawning. Already-registered instruments still export on the
/// periodic-reader cycle; only the lazy discovery of *new* `(name, kind)`
/// pairs is disabled.
fn spawn_discovery_task(
    meter: opentelemetry::metrics::Meter,
    inner: Arc<ExporterInner>,
    export_interval: Duration,
    stop: Arc<AtomicBool>,
) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::warn!(
            "OTLP metrics exporter installed outside a Tokio runtime; lazy discovery of new \
             (name, kind) pairs is disabled. Already-registered instruments still export on \
             the periodic-reader cycle."
        );
        return;
    };
    // Floor of 1s avoids tight loops on absurdly short configured intervals (test harnesses).
    let mut sleep_for = export_interval / 2;
    if sleep_for < Duration::from_secs(1) {
        sleep_for = Duration::from_secs(1);
    }
    handle.spawn(async move {
        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(sleep_for).await;
            if stop.load(Ordering::SeqCst) {
                return;
            }
            discover_and_register(&meter, &inner);
        }
    });
}

/// Snapshot the registry once and register an observable instrument for every `(name, kind)`
/// pair we have not seen yet. Existing instruments are left alone — their callbacks already
/// observe all matching label combinations on every OTel collection cycle.
fn discover_and_register(meter: &opentelemetry::metrics::Meter, inner: &Arc<ExporterInner>) {
    let interner = inner.registry.interner();

    // Counters.
    {
        let names: HashSet<String> = inner
            .registry
            .snapshot_counters()
            .into_iter()
            .map(|(key, _)| interner.resolve(key.name).to_owned())
            .collect();
        for name in names {
            if !mark_seen(inner, &name, "counter") {
                continue;
            }
            register_counter_observable(meter, Arc::clone(inner), name);
        }
    }

    // Gauges.
    {
        let names: HashSet<String> = inner
            .registry
            .snapshot_gauges()
            .into_iter()
            .map(|(key, _)| interner.resolve(key.name).to_owned())
            .collect();
        for name in names {
            if !mark_seen(inner, &name, "gauge") {
                continue;
            }
            register_gauge_observable(meter, Arc::clone(inner), name);
        }
    }

    // Histograms — register a triple of sum / count / bucket observables per name.
    {
        let names: HashSet<String> = inner
            .registry
            .snapshot_histograms()
            .into_iter()
            .map(|(key, _)| interner.resolve(key.name).to_owned())
            .collect();
        for name in names {
            if mark_seen(inner, &name, "histogram-sum") {
                register_histogram_sum_observable(meter, Arc::clone(inner), name.clone());
            }
            if mark_seen(inner, &name, "histogram-count") {
                register_histogram_count_observable(meter, Arc::clone(inner), name.clone());
            }
            if mark_seen(inner, &name, "histogram-bucket") {
                register_histogram_bucket_observable(meter, Arc::clone(inner), name);
            }
        }
    }
}

/// Insert `(name, role)` into the seen set. Returns `true` if the pair was new.
fn mark_seen(inner: &ExporterInner, name: &str, role: &'static str) -> bool {
    let Ok(mut guard) = inner.seen.lock() else {
        // Poisoned mutex — a previous registration panicked. Bail out rather than risk
        // double-registration; the exporter degrades to a no-op for new names.
        return false;
    };
    guard.insert((name.to_owned(), role))
}

fn register_counter_observable(
    meter: &opentelemetry::metrics::Meter,
    inner: Arc<ExporterInner>,
    name: String,
) {
    let target_name = name.clone();
    let _instrument = meter
        .u64_observable_counter(Cow::Owned(name))
        .with_callback(move |observer| {
            let interner = inner.registry.interner();
            for (key, counter) in inner.registry.snapshot_counters() {
                if interner.resolve(key.name) != target_name {
                    continue;
                }
                let attrs = inner.build_attributes(&key.labels, &[]);
                observer.observe(counter.get(), &attrs);
            }
        })
        .build();
}

fn register_gauge_observable(
    meter: &opentelemetry::metrics::Meter,
    inner: Arc<ExporterInner>,
    name: String,
) {
    let target_name = name.clone();
    let _instrument = meter
        .i64_observable_gauge(Cow::Owned(name))
        .with_callback(move |observer| {
            let interner = inner.registry.interner();
            for (key, gauge) in inner.registry.snapshot_gauges() {
                if interner.resolve(key.name) != target_name {
                    continue;
                }
                let attrs = inner.build_attributes(&key.labels, &[]);
                observer.observe(gauge.get(), &attrs);
            }
        })
        .build();
}

fn register_histogram_sum_observable(
    meter: &opentelemetry::metrics::Meter,
    inner: Arc<ExporterInner>,
    name: String,
) {
    let target_name = name.clone();
    let instrument_name = format!("{name}_sum");
    let _instrument = meter
        .f64_observable_counter(Cow::Owned(instrument_name))
        .with_callback(move |observer| {
            let interner = inner.registry.interner();
            for (key, histogram) in inner.registry.snapshot_histograms() {
                if interner.resolve(key.name) != target_name {
                    continue;
                }
                let snap = histogram.snapshot();
                let attrs = inner.build_attributes(&key.labels, &[]);
                observer.observe(snap.sum(), &attrs);
            }
        })
        .build();
}

fn register_histogram_count_observable(
    meter: &opentelemetry::metrics::Meter,
    inner: Arc<ExporterInner>,
    name: String,
) {
    let target_name = name.clone();
    let instrument_name = format!("{name}_count");
    let _instrument = meter
        .u64_observable_counter(Cow::Owned(instrument_name))
        .with_callback(move |observer| {
            let interner = inner.registry.interner();
            for (key, histogram) in inner.registry.snapshot_histograms() {
                if interner.resolve(key.name) != target_name {
                    continue;
                }
                let snap = histogram.snapshot();
                let attrs = inner.build_attributes(&key.labels, &[]);
                observer.observe(snap.observation_count(), &attrs);
            }
        })
        .build();
}

fn register_histogram_bucket_observable(
    meter: &opentelemetry::metrics::Meter,
    inner: Arc<ExporterInner>,
    name: String,
) {
    let target_name = name.clone();
    let instrument_name = format!("{name}_bucket");
    let _instrument = meter
        .u64_observable_counter(Cow::Owned(instrument_name))
        .with_callback(move |observer| {
            let interner = inner.registry.interner();
            for (key, histogram) in inner.registry.snapshot_histograms() {
                if interner.resolve(key.name) != target_name {
                    continue;
                }
                let snap = histogram.snapshot();
                let base_attrs = inner.build_attributes(&key.labels, &[]);
                for (upper_bound, cumulative) in snap.cumulative_buckets() {
                    let le_label = if upper_bound.is_finite() {
                        upper_bound.to_string()
                    } else {
                        "+Inf".to_owned()
                    };
                    let mut attrs = base_attrs.clone();
                    attrs.push(KeyValue::new("le", le_label));
                    observer.observe(cumulative, &attrs);
                }
            }
        })
        .build();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_cumulative_temporality_and_60s_interval() {
        let cfg = OtlpMetricsConfig::new("http://collector:4317");
        assert_eq!(cfg.endpoint, "http://collector:4317");
        assert_eq!(cfg.service_name, DEFAULT_SERVICE_NAME);
        assert_eq!(cfg.export_interval, DEFAULT_EXPORT_INTERVAL);
        assert_eq!(cfg.temporality, Temporality::Cumulative);
        assert!(cfg.allowlist.is_passthrough());
    }

    #[test]
    fn config_builder_overrides_apply() {
        let cfg = OtlpMetricsConfig::new("http://x:4317")
            .with_service_name("nebula-api")
            .with_export_interval(Duration::from_millis(250))
            .with_temporality(Temporality::Delta)
            .with_allowlist(LabelAllowlist::only(["status"]));
        assert_eq!(cfg.service_name, "nebula-api");
        assert_eq!(cfg.export_interval, Duration::from_millis(250));
        assert_eq!(cfg.temporality, Temporality::Delta);
        assert!(!cfg.allowlist.is_passthrough());
    }

    /// Smoke test: install pipeline against an unreachable endpoint (so the exporter builds
    /// but never actually flushes), confirm the guard can be constructed and dropped without
    /// panicking. Mirrors the trace-side `build_layer_then_shutdown_is_safe` test in
    /// `nebula-log`.
    #[tokio::test]
    async fn install_then_shutdown_is_safe() {
        let registry = Arc::new(MetricsRegistry::new());
        // Populate one of each kind so the discovery synchronous-first pass registers them.
        registry.counter("nebula_test_counter").unwrap().inc();
        registry.gauge("nebula_test_gauge").unwrap().set(7);
        registry
            .histogram("nebula_test_histogram")
            .unwrap()
            .observe(0.123);

        let cfg = OtlpMetricsConfig::new("http://127.0.0.1:1")
            .with_export_interval(Duration::from_mins(1));
        let mut guard = OtlpMetricsExporter::install(registry, cfg).expect("install succeeds");
        guard.shutdown();
    }

    #[test]
    fn cardinality_allowlist_strips_unlisted_keys_before_emission() {
        let registry = Arc::new(MetricsRegistry::new());
        let interner = registry.interner();
        let labels = interner.label_set(&[("execution_id", "uuid-1"), ("status", "ok")]);
        let inner = ExporterInner::new(Arc::clone(&registry), LabelAllowlist::only(["status"]));

        let attrs = inner.build_attributes(&labels, &[]);
        let keys: Vec<&str> = attrs.iter().map(|kv| kv.key.as_str()).collect();
        assert!(keys.contains(&"status"));
        assert!(!keys.contains(&"execution_id"));
    }

    #[test]
    fn build_attributes_appends_extras_after_resolved_pairs() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry.interner().label_set(&[("status", "ok")]);
        let inner = ExporterInner::new(registry, LabelAllowlist::all());

        let extras = [KeyValue::new("le", "0.5")];
        let attrs = inner.build_attributes(&labels, &extras);
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].key.as_str(), "status");
        assert_eq!(attrs[1].key.as_str(), "le");
    }
}
