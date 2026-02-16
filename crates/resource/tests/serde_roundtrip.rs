//! Property tests for serde JSON roundtrip of core types

use nebula_resource::health::{HealthState, HealthStatus};
use nebula_resource::lifecycle::Lifecycle;
use nebula_resource::pool::PoolConfig;
use nebula_resource::scope::{Scope, Strategy as ScopingStrategy};
use proptest::prelude::*;
use std::time::Duration;

/// Generate an arbitrary Lifecycle
fn arb_lifecycle() -> impl Strategy<Value = Lifecycle> {
    prop_oneof![
        Just(Lifecycle::Created),
        Just(Lifecycle::Initializing),
        Just(Lifecycle::Ready),
        Just(Lifecycle::InUse),
        Just(Lifecycle::Idle),
        Just(Lifecycle::Maintenance),
        Just(Lifecycle::Draining),
        Just(Lifecycle::Cleanup),
        Just(Lifecycle::Terminated),
        Just(Lifecycle::Failed),
    ]
}

/// Generate an arbitrary Scope
fn arb_scope() -> impl Strategy<Value = Scope> {
    prop_oneof![
        Just(Scope::Global),
        "[a-z0-9]{1,10}".prop_map(Scope::tenant),
        "[a-z0-9]{1,10}".prop_map(Scope::workflow),
        ("[a-z0-9]{1,10}", "[a-z0-9]{1,10}").prop_map(|(w, t)| Scope::workflow_in_tenant(w, t)),
        "[a-z0-9]{1,10}".prop_map(Scope::execution),
        (
            "[a-z0-9]{1,10}",
            "[a-z0-9]{1,10}",
            proptest::option::of("[a-z0-9]{1,10}")
        )
            .prop_map(|(e, w, t)| Scope::execution_in_workflow(e, w, t)),
        "[a-z0-9]{1,10}".prop_map(Scope::action),
        (
            "[a-z0-9]{1,10}",
            "[a-z0-9]{1,10}",
            proptest::option::of("[a-z0-9]{1,10}"),
            proptest::option::of("[a-z0-9]{1,10}"),
        )
            .prop_map(|(a, e, w, t)| Scope::action_in_execution(a, e, w, t)),
        ("[a-z]{1,10}", "[a-z0-9]{1,10}").prop_map(|(k, v)| Scope::custom(k, v)),
    ]
}

/// Generate an arbitrary HealthState
fn arb_health_state() -> impl Strategy<Value = HealthState> {
    prop_oneof![
        Just(HealthState::Healthy),
        ("[a-z ]{1,20}", 0.0f64..=1.0f64).prop_map(|(reason, impact)| HealthState::Degraded {
            reason,
            performance_impact: impact,
        }),
        ("[a-z ]{1,20}", any::<bool>()).prop_map(|(reason, recoverable)| {
            HealthState::Unhealthy {
                reason,
                recoverable,
            }
        }),
        Just(HealthState::Unknown),
    ]
}

/// Generate an arbitrary HealthStatus
fn arb_health_status() -> impl Strategy<Value = HealthStatus> {
    arb_health_state().prop_map(|state| HealthStatus {
        state,
        latency: None, // Duration doesn't roundtrip via default serde (it's not Serialize by default)
        metadata: std::collections::HashMap::new(),
    })
}

/// Generate an arbitrary PoolConfig
fn arb_pool_config() -> impl Strategy<Value = PoolConfig> {
    (
        1usize..100,
        1usize..100,
        1u64..600,
        1u64..3600,
        1u64..7200,
        1u64..300,
    )
        .prop_map(
            |(min, max_extra, acquire_s, idle_s, lifetime_s, validation_s)| PoolConfig {
                min_size: min,
                max_size: min + max_extra,
                acquire_timeout: Duration::from_secs(acquire_s),
                idle_timeout: Duration::from_secs(idle_s),
                max_lifetime: Duration::from_secs(lifetime_s),
                validation_interval: Duration::from_secs(validation_s),
                maintenance_interval: None,
                ..Default::default()
            },
        )
}

/// Generate an arbitrary Strategy
fn arb_strategy() -> impl Strategy<Value = ScopingStrategy> {
    prop_oneof![
        Just(ScopingStrategy::Strict),
        Just(ScopingStrategy::Hierarchical),
        Just(ScopingStrategy::Fallback),
    ]
}

proptest! {
    #[test]
    fn lifecycle_roundtrips(state in arb_lifecycle()) {
        let json = serde_json::to_string(&state).expect("serialize");
        let back: Lifecycle = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(state, back);
    }

    #[test]
    fn scope_roundtrips(scope in arb_scope()) {
        let json = serde_json::to_string(&scope).expect("serialize");
        let back: Scope = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(scope, back);
    }

    #[test]
    fn health_state_roundtrips(state in arb_health_state()) {
        let json = serde_json::to_string(&state).expect("serialize");
        let back: HealthState = serde_json::from_str(&json).expect("deserialize");
        // f64 may lose the last bit of precision during JSON roundtrip,
        // so compare structurally with tolerance for the float field.
        match (&state, &back) {
            (HealthState::Healthy, HealthState::Healthy) => {},
            (HealthState::Unknown, HealthState::Unknown) => {},
            (
                HealthState::Degraded { reason: r1, performance_impact: p1 },
                HealthState::Degraded { reason: r2, performance_impact: p2 },
            ) => {
                prop_assert_eq!(r1, r2);
                prop_assert!((p1 - p2).abs() < 1e-10, "performance_impact drift: {} vs {}", p1, p2);
            },
            (
                HealthState::Unhealthy { reason: r1, recoverable: rc1 },
                HealthState::Unhealthy { reason: r2, recoverable: rc2 },
            ) => {
                prop_assert_eq!(r1, r2);
                prop_assert_eq!(rc1, rc2);
            },
            _ => prop_assert!(false, "variant mismatch: {:?} vs {:?}", state, back),
        }
    }

    #[test]
    fn health_status_score_preserved_after_roundtrip(status in arb_health_status()) {
        let original_score = status.score();
        let json = serde_json::to_string(&status).expect("serialize");
        let back: HealthStatus = serde_json::from_str(&json).expect("deserialize");
        let roundtrip_score = back.score();
        // Score should be identical after roundtrip (it's derived from state)
        prop_assert!(
            (original_score - roundtrip_score).abs() < f64::EPSILON,
            "Score mismatch: {} vs {}",
            original_score,
            roundtrip_score
        );
    }

    #[test]
    fn pool_config_roundtrips(config in arb_pool_config()) {
        let json = serde_json::to_string(&config).expect("serialize");
        let back: PoolConfig = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(config.min_size, back.min_size);
        prop_assert_eq!(config.max_size, back.max_size);
        prop_assert_eq!(config.acquire_timeout, back.acquire_timeout);
        prop_assert_eq!(config.idle_timeout, back.idle_timeout);
        prop_assert_eq!(config.max_lifetime, back.max_lifetime);
        prop_assert_eq!(config.validation_interval, back.validation_interval);
    }

    #[test]
    fn strategy_roundtrips(strategy in arb_strategy()) {
        let json = serde_json::to_string(&strategy).expect("serialize");
        let back: ScopingStrategy = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(strategy, back);
    }
}

/// Verify that Lifecycle JSON output is a simple string (not an object)
#[test]
fn lifecycle_json_is_simple_string() {
    let json = serde_json::to_string(&Lifecycle::Ready).unwrap();
    // Should be a quoted string like "Ready", not an object
    assert!(json.starts_with('"') && json.ends_with('"'));
    assert_eq!(json, "\"Ready\"");
}

/// Verify that Scope::Global serializes cleanly
#[test]
fn global_scope_serialization() {
    let json = serde_json::to_string(&Scope::Global).unwrap();
    let back: Scope = serde_json::from_str(&json).unwrap();
    assert_eq!(back, Scope::Global);
}

// ---------------------------------------------------------------------------
// Corrupt / malformed input tests — garbage in must produce Err, not panic
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_rejects_unknown_variant() {
    let result = serde_json::from_str::<Lifecycle>("\"NotAState\"");
    assert!(result.is_err(), "unknown variant should produce Err");
}

#[test]
fn lifecycle_rejects_number() {
    let result = serde_json::from_str::<Lifecycle>("42");
    assert!(result.is_err(), "number should produce Err for Lifecycle");
}

#[test]
fn lifecycle_rejects_null() {
    let result = serde_json::from_str::<Lifecycle>("null");
    assert!(result.is_err(), "null should produce Err for Lifecycle");
}

#[test]
fn scope_rejects_empty_object() {
    let result = serde_json::from_str::<Scope>("{}");
    assert!(result.is_err(), "empty object should produce Err for Scope");
}

#[test]
fn scope_rejects_unknown_variant() {
    let result = serde_json::from_str::<Scope>("{\"UnknownScope\": {}}");
    assert!(
        result.is_err(),
        "unknown variant should produce Err for Scope"
    );
}

#[test]
fn health_state_rejects_wrong_type() {
    let result = serde_json::from_str::<HealthState>(
        r#"{"Degraded":{"reason":"x","performance_impact":"not_a_number"}}"#,
    );
    assert!(
        result.is_err(),
        "string where f64 expected should produce Err"
    );
}

#[test]
fn health_state_rejects_null() {
    let result = serde_json::from_str::<HealthState>("null");
    assert!(result.is_err(), "null should produce Err for HealthState");
}

#[test]
fn pool_config_rejects_null() {
    let result = serde_json::from_str::<PoolConfig>("null");
    assert!(result.is_err(), "null should produce Err for PoolConfig");
}

#[test]
fn pool_config_rejects_truncated_json() {
    let result = serde_json::from_str::<PoolConfig>(r#"{"min_size": 1, "max_si"#);
    assert!(result.is_err(), "truncated JSON should produce Err");
}

#[test]
fn strategy_rejects_unknown_variant() {
    let result = serde_json::from_str::<ScopingStrategy>("\"NonExistent\"");
    assert!(
        result.is_err(),
        "unknown variant should produce Err for Strategy"
    );
}

#[test]
fn strategy_rejects_number() {
    let result = serde_json::from_str::<ScopingStrategy>("99");
    assert!(result.is_err(), "number should produce Err for Strategy");
}
