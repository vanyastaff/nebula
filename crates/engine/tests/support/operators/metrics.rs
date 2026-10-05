//! Counter snapshots that never create a missing metric while observing it.

use std::collections::BTreeMap;

use nebula_metrics::MetricsRegistry;

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct CounterObservation {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub value: u64,
}

pub(super) fn snapshot(registry: &MetricsRegistry) -> Vec<CounterObservation> {
    let interner = registry.interner();
    registry
        .snapshot_counters()
        .into_iter()
        .map(|(key, counter)| CounterObservation {
            name: interner.resolve(key.name).to_owned(),
            labels: key
                .labels
                .iter()
                .map(|(name, value)| {
                    (
                        interner.resolve(name).to_owned(),
                        interner.resolve(value).to_owned(),
                    )
                })
                .collect(),
            value: counter.get(),
        })
        .collect()
}

pub(super) fn outcome_delta(
    before: &[CounterObservation],
    after: &[CounterObservation],
    name: &str,
    backend: &str,
    outcome: &str,
) -> u64 {
    let count = |rows: &[CounterObservation]| {
        rows.iter()
            .filter(|row| {
                row.name == name
                    && row
                        .labels
                        .get("backend")
                        .is_some_and(|label| label == backend)
                    && row
                        .labels
                        .get("outcome")
                        .is_some_and(|label| label == outcome)
            })
            .map(|row| row.value)
            .sum::<u64>()
    };
    count(after)
        .checked_sub(count(before))
        .expect("monotonic outcome counter")
}
