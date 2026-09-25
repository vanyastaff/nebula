use std::time::Duration;

use serde_json::json;

use super::*;
use crate::{ErrorKind, topology::store::PoolStrategy};

fn pool(value: serde_json::Value) -> Result<PoolConfig, Error> {
    serde_json::from_value::<PoolSettings>(value)
        .map_err(|error| Error::permanent(error.to_string()))?
        .into_config()
}

#[test]
fn empty_pool_settings_keep_every_default() {
    let config = pool(json!({})).expect("empty settings are valid");
    let defaults = PoolConfig::default();
    assert_eq!(config.max_size, defaults.max_size);
    assert_eq!(config.create_timeout, defaults.create_timeout);
    assert_eq!(config.idle_timeout, defaults.idle_timeout);
}

#[test]
fn pool_settings_map_every_field() {
    let config = pool(json!({
        "min_size": 2,
        "max_size": 40,
        "idle_timeout_ms": 0,
        "max_lifetime_ms": 600_000,
        "create_timeout_ms": 1_500,
        "strategy": "fifo",
        "warmup": "staggered",
        "warmup_interval_ms": 250,
        "test_on_checkout": true,
        "maintenance_interval_ms": 5_000,
        "max_concurrent_creates": 8
    }))
    .expect("valid settings");
    assert_eq!((config.min_size, config.max_size), (2, 40));
    assert_eq!(config.idle_timeout, None, "0 disables idle eviction");
    assert_eq!(config.max_lifetime, Some(Duration::from_mins(10)));
    assert_eq!(config.create_timeout, Duration::from_millis(1_500));
    assert_eq!(config.strategy, PoolStrategy::Fifo);
    assert!(matches!(
        config.warmup,
        WarmupStrategy::Staggered { interval } if interval == Duration::from_millis(250)
    ));
    assert!(config.test_on_checkout);
    assert_eq!(config.maintenance_interval, Duration::from_secs(5));
    assert_eq!(config.max_concurrent_creates, 8);
}

#[test]
fn unknown_or_mistyped_fields_are_rejected() {
    assert!(
        pool(json!({ "max_sizee": 4 })).is_err(),
        "typo must not be ignored"
    );
    assert!(pool(json!({ "max_size": "4" })).is_err());
    assert!(pool(json!({ "warmup": "eager" })).is_err());
}

#[test]
fn zero_budgets_are_rejected_before_any_pool_exists() {
    for (field, value) in [
        ("create_timeout_ms", json!({ "create_timeout_ms": 0 })),
        (
            "maintenance_interval_ms",
            json!({ "maintenance_interval_ms": 0 }),
        ),
        (
            "max_concurrent_creates",
            json!({ "max_concurrent_creates": 0 }),
        ),
    ] {
        let error = pool(value).expect_err(field);
        assert_eq!(error.kind(), &ErrorKind::Permanent, "{field}");
    }
}

/// A maintenance interval the reaper's timer could not be armed with is
/// refused as a setting, not clamped in silence.
#[test]
fn a_maintenance_interval_past_the_ceiling_is_rejected() {
    let ceiling = u64::try_from(MAX_MAINTENANCE_INTERVAL.as_millis()).expect("fits in u64");
    let config = pool(json!({ "maintenance_interval_ms": ceiling })).expect("the ceiling itself");
    assert_eq!(config.maintenance_interval, MAX_MAINTENANCE_INTERVAL);
    for ms in [ceiling + 1, u64::MAX] {
        let error = pool(json!({ "maintenance_interval_ms": ms })).expect_err("past the ceiling");
        assert_eq!(error.kind(), &ErrorKind::Permanent, "{ms}");
        assert!(
            error.to_string().contains("maintenance_interval_ms"),
            "{error}"
        );
    }
}

#[test]
fn resident_settings_default_and_validate() {
    let defaults = ResidentSettings::default().into_config().expect("defaults");
    assert_eq!(
        defaults.create_timeout,
        ResidentConfig::default().create_timeout
    );
    let custom = serde_json::from_value::<ResidentSettings>(json!({
        "recreate_on_failure": true,
        "create_timeout_ms": 2_000
    }))
    .expect("valid")
    .into_config()
    .expect("valid");
    assert!(custom.recreate_on_failure);
    assert_eq!(custom.create_timeout, Duration::from_secs(2));
    assert!(
        ResidentSettings {
            create_timeout_ms: Some(0),
            ..ResidentSettings::default()
        }
        .into_config()
        .is_err()
    );
}

#[test]
fn warmup_interval_must_match_the_warmup_mode() {
    assert!(
        pool(json!({ "warmup": "staggered" })).is_err(),
        "staggered warmup needs its interval"
    );
    assert!(
        pool(json!({ "warmup": "parallel", "warmup_interval_ms": 100 })).is_err(),
        "an interval for a non-staggered warmup must not be silently ignored"
    );
    assert!(pool(json!({ "warmup_interval_ms": 100 })).is_err());
    assert!(pool(json!({ "warmup": "staggered", "warmup_interval_ms": 0 })).is_err());
}

#[test]
fn bounded_settings_require_an_explicit_mode() {
    assert_eq!(
        serde_json::from_value::<BoundedSettings>(json!({ "mode": "capped", "max_concurrent": 3 }))
            .expect("valid"),
        BoundedSettings::capped(3)
    );
    assert_eq!(
        serde_json::from_value::<BoundedSettings>(json!({ "mode": "exclusive" })).expect("valid"),
        BoundedSettings::uncapped(BoundedModeSetting::Exclusive)
    );
    assert!(
        serde_json::from_value::<BoundedSettings>(json!({})).is_err(),
        "mode is required"
    );
    assert!(serde_json::from_value::<BoundedSettings>(json!({ "mode": "any" })).is_err());
}

#[test]
fn settings_schemas_build_and_expose_modes_as_selects() {
    use nebula_schema::{Property, schema_of};

    let pool = schema_of::<PoolSettings>().expect("pool settings schema");
    let strategy = pool
        .properties()
        .iter()
        .find(|property| property.key().as_str() == "strategy")
        .expect("strategy is published");
    assert!(
        matches!(strategy, Property::Select(_)),
        "a mode is a select, not free text"
    );

    schema_of::<ResidentSettings>().expect("resident settings schema");
    let bounded = schema_of::<BoundedSettings>().expect("bounded settings schema");
    assert!(
        bounded
            .properties()
            .iter()
            .any(|property| property.key().as_str() == "max_concurrent")
    );
}
