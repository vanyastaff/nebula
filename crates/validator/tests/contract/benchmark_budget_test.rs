//! Benchmark budget contract tests
//!
//! These tests enforce the integrity of performance budgets and memory layout
//! invariants. They do NOT measure actual runtime performance (which is
//! machine-dependent) — that is done by Criterion benchmarks + CI scripts.
//!
//! Instead, these tests validate:
//! - Budget fixture file is well-formed and complete
//! - Memory layout contracts (ValidationError size)
//! - Budget values are sane (positive, within bounds)

use serde_json::Value;
use std::fs;

// ============================================================================
// BUDGET FIXTURE INTEGRITY
// ============================================================================

fn load_budgets() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/perf/benchmark_budgets_v1.json"
    );
    let content = fs::read_to_string(path).expect("benchmark_budgets_v1.json must exist");
    serde_json::from_str(&content).expect("benchmark_budgets_v1.json must be valid JSON")
}

#[test]
fn budget_fixture_has_metadata() {
    let budgets = load_budgets();
    let meta = &budgets["metadata"];
    assert!(meta["version"].is_string(), "metadata.version required");
    assert!(
        meta["description"].is_string(),
        "metadata.description required"
    );
    assert!(meta["policy"].is_string(), "metadata.policy required");
}

#[test]
fn budget_fixture_has_all_categories() {
    let budgets = load_budgets();
    let b = &budgets["budgets"];

    let required_categories = ["validators", "combinators", "real_world", "memory"];
    for cat in required_categories {
        assert!(b[cat].is_object(), "budgets.{cat} must be an object");
    }
}

#[test]
fn budget_fixture_has_change_policy() {
    let budgets = load_budgets();
    let policy = &budgets["change_policy"];
    assert!(
        policy["relaxation"].is_string(),
        "change_policy.relaxation required"
    );
    assert!(
        policy["tightening"].is_string(),
        "change_policy.tightening required"
    );
    assert!(
        policy["new_budgets"].is_string(),
        "change_policy.new_budgets required"
    );
}

// ============================================================================
// BUDGET VALUE VALIDATION
// ============================================================================

#[expect(
    clippy::excessive_nesting,
    reason = "recursive helper function with nested if-let chains naturally requires this depth"
)]
#[test]
fn all_ns_budgets_are_positive_and_bounded() {
    let budgets = load_budgets();
    let max_budget_ns = 10_000; // 10µs is the absolute max for any single operation

    fn check_ns_values(obj: &Value, path: &str, max: u64) {
        if let Some(map) = obj.as_object() {
            for (key, value) in map {
                let current_path = format!("{path}.{key}");
                if let Some(max_ns) = value.get("max_ns") {
                    let ns = max_ns.as_u64().unwrap_or_else(|| {
                        panic!("{current_path}.max_ns must be a positive integer")
                    });
                    assert!(ns > 0, "{current_path}.max_ns must be > 0, got {ns}");
                    assert!(
                        ns <= max,
                        "{current_path}.max_ns = {ns} exceeds absolute max {max}"
                    );
                } else if value.is_object() {
                    check_ns_values(value, &current_path, max);
                }
            }
        }
    }

    check_ns_values(&budgets["budgets"], "budgets", max_budget_ns);
}

#[test]
fn all_byte_budgets_are_positive() {
    let budgets = load_budgets();
    let memory = &budgets["budgets"]["memory"];

    if let Some(map) = memory.as_object() {
        for (key, value) in map {
            if key == "description" {
                continue;
            }
            if let Some(max_bytes) = value.get("max_bytes") {
                let bytes = max_bytes
                    .as_u64()
                    .unwrap_or_else(|| panic!("memory.{key}.max_bytes must be a positive integer"));
                assert!(bytes > 0, "memory.{key}.max_bytes must be > 0");
            }
        }
    }
}

// ============================================================================
// VALIDATOR SUCCESS PATH BUDGETS
// ============================================================================

#[test]
fn success_path_budgets_cover_all_validator_families() {
    let budgets = load_budgets();
    let success = &budgets["budgets"]["validators"]["success_path"];

    let required = [
        "length_check",
        "not_empty",
        "pattern_match",
        "char_class",
        "email",
        "url",
    ];
    for name in required {
        assert!(
            success[name].is_object(),
            "success_path.{name} budget missing"
        );
    }
}

#[test]
fn error_path_budgets_cover_required_scenarios() {
    let budgets = load_budgets();
    let error = &budgets["budgets"]["validators"]["error_path"];

    let required = ["simple_error", "error_with_params", "error_with_nested"];
    for name in required {
        assert!(error[name].is_object(), "error_path.{name} budget missing");
    }
}

// ============================================================================
// COMBINATOR BUDGETS
// ============================================================================

#[test]
fn combinator_budgets_cover_core_operations() {
    let budgets = load_budgets();
    let comb = &budgets["budgets"]["combinators"];

    let required = [
        "and_two",
        "and_five",
        "and_ten",
        "or_two",
        "not",
        "when_skipped",
        "cached_hit",
        "cached_miss",
    ];
    for name in required {
        assert!(comb[name].is_object(), "combinators.{name} budget missing");
    }
}

#[test]
fn combinator_budget_ordering_is_consistent() {
    let budgets = load_budgets();
    let comb = &budgets["budgets"]["combinators"];

    // AND chain cost should increase with depth
    let and_two = comb["and_two"]["max_ns"].as_u64().unwrap();
    let and_five = comb["and_five"]["max_ns"].as_u64().unwrap();
    let and_ten = comb["and_ten"]["max_ns"].as_u64().unwrap();
    assert!(
        and_two <= and_five,
        "and_two ({and_two}) should be <= and_five ({and_five})"
    );
    assert!(
        and_five <= and_ten,
        "and_five ({and_five}) should be <= and_ten ({and_ten})"
    );

    // Cache hit should be cheaper than cache miss
    let hit = comb["cached_hit"]["max_ns"].as_u64().unwrap();
    let miss = comb["cached_miss"]["max_ns"].as_u64().unwrap();
    assert!(
        hit < miss,
        "cached_hit ({hit}) should be < cached_miss ({miss})"
    );
}

// ============================================================================
// MEMORY LAYOUT CONTRACT
// ============================================================================

#[test]
fn validation_error_size_matches_budget() {
    let budgets = load_budgets();
    let expected = budgets["budgets"]["memory"]["validation_error_size"]["max_bytes"]
        .as_u64()
        .unwrap() as usize;

    let actual = std::mem::size_of::<nebula_validator::foundation::ValidationError>();
    assert_eq!(
        actual, expected,
        "ValidationError size changed: expected {expected} bytes, got {actual} bytes. \
         Update benchmark_budgets_v1.json if this is intentional."
    );
}

#[test]
fn error_extras_pointer_fits_budget() {
    let budgets = load_budgets();
    let expected = budgets["budgets"]["memory"]["error_extras_box"]["max_bytes"]
        .as_u64()
        .unwrap() as usize;

    // Option<Box<T>> is pointer-sized
    let actual = std::mem::size_of::<Option<Box<u8>>>();
    assert_eq!(
        actual, expected,
        "Option<Box<ErrorExtras>> size changed: expected {expected} bytes, got {actual} bytes"
    );
}

// ============================================================================
// REAL-WORLD BUDGET COMPLETENESS
// ============================================================================

#[test]
fn real_world_budgets_cover_form_scenarios() {
    let budgets = load_budgets();
    let rw = &budgets["budgets"]["real_world"];

    let required = [
        "username_valid",
        "email_valid",
        "form_3_fields_valid",
        "form_3_fields_one_invalid",
    ];
    for name in required {
        assert!(rw[name].is_object(), "real_world.{name} budget missing");
    }
}

#[test]
fn valid_path_cheaper_than_error_path() {
    let budgets = load_budgets();
    let rw = &budgets["budgets"]["real_world"];

    let valid = rw["form_3_fields_valid"]["max_ns"].as_u64().unwrap();
    let invalid = rw["form_3_fields_one_invalid"]["max_ns"].as_u64().unwrap();
    assert!(
        valid < invalid,
        "form valid ({valid} ns) should be cheaper than invalid ({invalid} ns)"
    );
}
