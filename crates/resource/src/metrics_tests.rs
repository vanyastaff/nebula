use super::*;

#[test]
fn counters_start_at_zero() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();
    let snap = metrics.snapshot();
    assert_eq!(snap.acquire_total, 0);
    assert_eq!(snap.acquire_errors, 0);
    assert_eq!(snap.release_total, 0);
    assert_eq!(snap.release_errors, 0);
    assert_eq!(snap.create_total, 0);
    assert_eq!(snap.destroy_total, 0);
}

#[test]
fn record_and_snapshot() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();
    metrics.record_acquire();
    metrics.record_acquire();
    metrics.record_acquire_error();
    metrics.record_release();
    metrics.record_release_error();
    metrics.record_release_error();
    metrics.record_create();
    metrics.record_create();
    metrics.record_create();
    metrics.record_destroy();

    let snap = metrics.snapshot();
    assert_eq!(snap.acquire_total, 2);
    assert_eq!(snap.acquire_errors, 1);
    assert_eq!(snap.release_total, 1);
    assert_eq!(snap.release_errors, 2);
    assert_eq!(snap.create_total, 3);
    assert_eq!(snap.destroy_total, 1);
}

#[test]
fn clones_share_counters() {
    let registry = MetricsRegistry::new();
    let m1 = ResourceOpsMetrics::new(&registry).unwrap();
    let m2 = m1.clone();

    m1.record_acquire();
    m2.record_acquire();

    assert_eq!(m1.snapshot().acquire_total, 2);
    assert_eq!(m2.snapshot().acquire_total, 2);
}

#[test]
fn backed_by_registry() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();
    metrics.record_create();
    metrics.record_create();

    // Read directly from registry to verify shared backing.
    let counter = registry.counter(NEBULA_RESOURCE_CREATE_TOTAL).unwrap();
    assert_eq!(counter.get(), 2);
}

#[test]
fn refresh_attempts_is_sum_of_outcomes() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();

    // Four dispatches: two completed, one failed, one abandoned. One
    // outcome per dispatch, so attempts is the sum of all four labels.
    metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Success);
    metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Success);
    metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Failed);
    metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Abandoned);

    let snap = metrics.snapshot();
    let o = snap.slot_refresh_outcomes;
    assert_eq!(o.success, 2);
    assert_eq!(o.failed, 1);
    assert_eq!(o.timed_out, 0);
    assert_eq!(o.abandoned, 1);
    assert_eq!(
        o.success + o.failed + o.timed_out + o.abandoned,
        4,
        "attempts == Σ outcomes"
    );
}

#[test]
fn revoke_outcome_split_counts_timed_out() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();

    metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::Success);
    metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::TimedOut);
    metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::Abandoned);

    let snap = metrics.snapshot();
    let o = snap.slot_revoke_outcomes;
    assert_eq!(o.success, 1);
    assert_eq!(o.failed, 0);
    assert_eq!(o.timed_out, 1);
    assert_eq!(o.abandoned, 1);
}

#[test]
fn deferred_observation_is_separate_from_terminal_attempts() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();

    metrics.record_slot_refresh_deferred();
    metrics.record_slot_revoke_deferred();
    metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Success);
    metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::Abandoned);

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.slot_refresh_deferred, 1);
    assert_eq!(snapshot.slot_revoke_deferred, 1);
    assert_eq!(snapshot.slot_refresh_outcomes.success, 1);
    assert_eq!(snapshot.slot_refresh_outcomes.abandoned, 0);
    assert_eq!(snapshot.slot_revoke_outcomes.success, 0);
    assert_eq!(snapshot.slot_revoke_outcomes.abandoned, 1);
}

/// The per-`outcome` split must reach the shared registry — the same
/// `(name, outcome=<value>)` series the manager wrote is observable
/// through a sibling `counter_labeled` handle and is enumerated by
/// `snapshot_counters` (what an exporter scrapes).
#[test]
fn outcome_split_is_registry_bound() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();

    metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Success);
    metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Failed);
    metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Abandoned);
    metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::TimedOut);

    // Sibling handle on the same registry sees the same atomic.
    let refresh_success = registry
        .counter_labeled(
            NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
            &outcome_label(&registry, rotation_outcome::SUCCESS),
        )
        .unwrap();
    assert_eq!(
        refresh_success.get(),
        1,
        "refresh success must be registry-bound"
    );

    let refresh_abandoned = registry
        .counter_labeled(
            NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
            &outcome_label(&registry, rotation_outcome::ABANDONED),
        )
        .unwrap();
    assert_eq!(
        refresh_abandoned.get(),
        1,
        "refresh abandonment must be registry-bound"
    );

    let revoke_timed_out = registry
        .counter_labeled(
            NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
            &outcome_label(&registry, rotation_outcome::TIMED_OUT),
        )
        .unwrap();
    assert_eq!(
        revoke_timed_out.get(),
        1,
        "revoke timed_out must be registry-bound"
    );

    // And the series is enumerated by the exporter-facing snapshot.
    let name_spur = registry
        .interner()
        .intern(NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL);
    let labeled_series = registry
        .snapshot_counters()
        .into_iter()
        .filter(|(k, _)| k.name == name_spur && !k.labels.is_empty())
        .count();
    assert_eq!(
        labeled_series, 4,
        "all four outcome series of the refresh attempts counter must be registered"
    );
}

// ── acquire wait-time histogram + waited/timed-out counters ────────────

#[test]
fn acquire_wait_starts_at_zero() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();
    let snap = metrics.snapshot().acquire_wait;
    assert_eq!(snap.waited_count, 0);
    assert_eq!(snap.timed_out_count, 0);
    assert_eq!(
        snap.bucket_counts(),
        [0u64; ACQUIRE_WAIT_BUCKET_COUNT].as_slice()
    );
}

#[test]
fn record_acquire_wait_falls_in_the_expected_bucket() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();

    // 5µs: well under the 100µs first bucket — the hot pooled/resident-hit
    // path (~2µs warm-hit baseline; see benches/acquire.rs) must resolve
    // here, not collapse into a coarser bucket.
    metrics.record_acquire_wait(Duration::from_micros(5), false);
    // 50ms: falls in the <=100ms bucket (index 3).
    metrics.record_acquire_wait(Duration::from_millis(50), false);
    // 20s: exceeds every finite bound — the `> 10s` overflow bucket.
    metrics.record_acquire_wait(Duration::from_secs(20), false);

    let snap = metrics.snapshot().acquire_wait;
    assert_eq!(
        snap.bucket_counts()[0],
        1,
        "5µs must land in the <=100µs bucket"
    );
    assert_eq!(
        snap.bucket_counts()[3],
        1,
        "50ms must land in the <=100ms bucket"
    );
    assert_eq!(
        snap.bucket_counts()[ACQUIRE_WAIT_BUCKET_COUNT - 1],
        1,
        "20s must land in the >10s overflow bucket"
    );
    assert_eq!(
        snap.bucket_counts().iter().sum::<u64>(),
        3,
        "every observation must land in exactly one bucket"
    );
    assert_eq!(
        snap.buckets().last(),
        Some((None, 1)),
        "the last paired entry is the overflow bucket, with no finite upper bound"
    );
}

#[test]
fn record_acquire_wait_tracks_waited() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();

    // Below the immediate threshold: not "waited".
    metrics.record_acquire_wait(Duration::from_micros(5), false);
    // Above it: counts as "waited".
    metrics.record_acquire_wait(Duration::from_millis(1), false);

    let snap = metrics.snapshot().acquire_wait;
    assert_eq!(
        snap.waited_count, 1,
        "only the above-threshold acquire counts as waited"
    );
}

#[test]
fn record_acquire_wait_counts_timed_out_only_when_flagged() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();

    metrics.record_acquire_wait(Duration::from_millis(1), false);
    metrics.record_acquire_wait(Duration::from_millis(1), true);

    let snap = metrics.snapshot().acquire_wait;
    assert_eq!(snap.timed_out_count, 1);
}

#[test]
fn acquire_wait_histogram_is_registry_bound() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();
    metrics.record_acquire_wait(Duration::from_millis(1), false);

    // A sibling handle built the same way (same name, empty label set,
    // same custom buckets) must see the same underlying series — proving
    // the empty-`LabelSet` construction in `ResourceOpsMetrics::new` is
    // registry-bound and not a private, unexported histogram.
    let sibling = registry
        .histogram_with_buckets_labeled(
            NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
            &LabelSet::empty(),
            ACQUIRE_WAIT_BUCKET_BOUNDS_SECONDS.to_vec(),
        )
        .unwrap();
    assert_eq!(
        sibling.count(),
        1,
        "acquire-wait histogram must be registry-bound"
    );

    // And an unlabeled lookup resolves to the very same `MetricKey`
    // (empty label set == unlabeled) — this is *why* the custom-bucket
    // empty-labeled construction works as the crate's de facto unlabeled
    // custom-bucket entry point.
    let name_spur = registry
        .interner()
        .intern(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS);
    let unlabeled_series = registry
        .snapshot_histograms()
        .into_iter()
        .filter(|(k, _)| k.name == name_spur && k.labels.is_empty())
        .count();
    assert_eq!(
        unlabeled_series, 1,
        "the acquire-wait histogram must be the sole unlabeled series under its name"
    );
}

// ── hold-deadline-exceeded counter ──────────────────────────────────────

#[test]
fn record_hold_deadline_exceeded_increments() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).unwrap();
    assert_eq!(metrics.snapshot().hold_deadline_exceeded, 0);
    metrics.record_hold_deadline_exceeded();
    metrics.record_hold_deadline_exceeded();
    assert_eq!(metrics.snapshot().hold_deadline_exceeded, 2);
}
