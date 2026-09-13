//! Failed derived construction must return diagnostics without unwinding.

use std::sync::atomic::{AtomicUsize, Ordering};

use nebula_schema::{
    EnumSelect, HasSchema, Schema, ValidSchema, ValidationError, ValidationReport, schema_of,
};

#[derive(EnumSelect)]
#[expect(
    dead_code,
    reason = "schema construction inspects the declared options"
)]
enum Choice {
    Available,
}

#[derive(Schema)]
#[expect(
    dead_code,
    reason = "schema construction inspects the declared default"
)]
struct InvalidDefault {
    #[field(enum_select, default = "missing")]
    choice: Choice,
}

#[test]
fn runtime_schema_lint_does_not_unwind() {
    let result = std::panic::catch_unwind(InvalidDefault::schema);
    assert!(
        result.is_ok(),
        "invalid derived schemas must return a report"
    );
    let report = result.unwrap().unwrap_err();
    assert_eq!(report.errors().count(), 1);
    let error = report.errors().next().unwrap();
    assert_eq!(error.code(), "default.type_mismatch");
    assert_eq!(error.path().to_string(), "/choice");
}

#[test]
fn repeated_failed_construction_returns_the_same_report() {
    let first = schema_of::<InvalidDefault>().unwrap_err();
    let second = schema_of::<InvalidDefault>().unwrap_err();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap(),
    );
    assert_eq!(
        first.errors().next().unwrap().code(),
        "default.type_mismatch"
    );
}

#[derive(Schema)]
#[expect(dead_code, reason = "only nested schema construction is exercised")]
struct NestedInvalid {
    child: InvalidDefault,
}

#[derive(Schema)]
#[expect(dead_code, reason = "only nested schema construction is exercised")]
struct ListInvalid {
    children: Vec<InvalidDefault>,
}

#[derive(Schema)]
#[expect(dead_code, reason = "only nested schema construction is exercised")]
enum NewtypeInvalid {
    Child(InvalidDefault),
}

#[derive(Schema)]
#[expect(dead_code, reason = "only nested schema construction is exercised")]
enum StructVariantInvalid {
    Child { child: InvalidDefault },
}

fn assert_nested_report<T: HasSchema>() {
    let child = schema_of::<InvalidDefault>().unwrap_err();
    let parent = schema_of::<T>().unwrap_err();
    assert_eq!(
        serde_json::to_value(parent).unwrap(),
        serde_json::to_value(child).unwrap(),
        "nested construction must preserve the original typed report",
    );
}

#[test]
fn object_propagates_nested_failure() {
    assert_nested_report::<NestedInvalid>();
}

#[test]
fn list_propagates_nested_failure() {
    assert_nested_report::<ListInvalid>();
}

#[test]
fn union_newtype_propagates_nested_failure() {
    assert_nested_report::<NewtypeInvalid>();
}

#[test]
fn union_struct_variant_propagates_nested_failure() {
    assert_nested_report::<StructVariantInvalid>();
}

#[derive(Schema)]
#[expect(dead_code, reason = "only checked pattern construction is exercised")]
struct InvalidPattern {
    #[validate(pattern = "[")]
    value: String,
}

#[derive(Schema)]
#[expect(dead_code, reason = "only checked pattern construction is exercised")]
enum InvalidVariantPattern {
    Item {
        #[validate(pattern = "[")]
        value: String,
    },
}

#[test]
fn invalid_pattern_construction_returns_a_typed_report() {
    for report in [
        schema_of::<InvalidPattern>().unwrap_err(),
        schema_of::<InvalidVariantPattern>().unwrap_err(),
    ] {
        assert_eq!(report.errors().count(), 1);
        let error = report.errors().next().unwrap();
        assert_eq!(error.code(), "schema.invalid_pattern");
        assert!(std::error::Error::source(error).is_some());
    }
}

static NESTED_CALLS: AtomicUsize = AtomicUsize::new(0);

struct CountedFailure;

impl HasSchema for CountedFailure {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        NESTED_CALLS.fetch_add(1, Ordering::SeqCst);
        Err(ValidationError::builder("nested.construction_failed")
            .build()
            .into())
    }
}

#[derive(Schema)]
#[expect(
    dead_code,
    reason = "the cache is exercised through schema construction"
)]
struct CachedFailure {
    child: CountedFailure,
}

#[test]
fn concurrent_failed_calls_initialize_the_cache_once() {
    let reports = std::thread::scope(|scope| {
        let calls: [_; 8] =
            std::array::from_fn(|_| scope.spawn(|| schema_of::<CachedFailure>().unwrap_err()));
        calls
            .into_iter()
            .map(|call| call.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(NESTED_CALLS.load(Ordering::SeqCst), 1);
    let expected = serde_json::to_value(&reports[0]).unwrap();
    assert_eq!(
        reports[0].errors().next().unwrap().code(),
        "nested.construction_failed"
    );
    for report in reports {
        assert_eq!(serde_json::to_value(report).unwrap(), expected);
    }
    assert_eq!(
        serde_json::to_value(schema_of::<CachedFailure>().unwrap_err()).unwrap(),
        expected,
    );
    assert_eq!(NESTED_CALLS.load(Ordering::SeqCst), 1);
}
