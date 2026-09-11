use nebula_schema::{Field, Schema, field_key};
use nebula_validator::{MAX_RULE_DEPTH, Rule, RuleBuildError};

fn boundary_rule() -> Rule {
    let mut rule = Rule::email();
    for _ in 1..MAX_RULE_DEPTH {
        rule = Rule::not(rule).unwrap();
    }
    rule
}

#[test]
fn over_limit_rule_cannot_reach_schema_admission() {
    let error = Rule::not(boundary_rule()).unwrap_err();
    assert_eq!(
        error,
        RuleBuildError::DepthLimit {
            limit: MAX_RULE_DEPTH,
        }
    );
}

#[test]
fn schema_builder_accepts_rule_at_depth_boundary() {
    let result = Schema::builder()
        .add(Field::string(field_key!("email")).with_rule(boundary_rule()))
        .build();
    assert!(
        result.is_ok(),
        "boundary rule should be admitted: {result:?}"
    );
}
