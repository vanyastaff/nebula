//! Prepared proof custody follows the retained admission, not descriptor equality.

use nebula_schema::{InputContract, schema_type};
use serde_json::json;

#[schema_type(input)]
struct CustodyInput(String);

#[test]
fn cloned_contract_retains_custody_but_independent_equal_admission_does_not() {
    let original = InputContract::for_type::<CustodyInput>().unwrap();
    let cloned = original.clone();
    let independent = InputContract::for_type::<CustodyInput>().unwrap();
    assert_eq!(original.graph(), independent.graph());
    assert_eq!(
        original.semantic_commitment(),
        independent.semantic_commitment()
    );

    let proof = original.validate_data(json!("ready")).unwrap();
    assert!(proof.belongs_to(&original));
    assert!(proof.belongs_to(&cloned));
    assert!(!proof.belongs_to(&independent));
    let decoded = proof.into_typed::<CustodyInput>(&cloned).unwrap();
    assert_eq!(decoded.0, "ready");

    let error = original
        .validate_data(json!("ready"))
        .unwrap()
        .into_typed::<CustodyInput>(&independent)
        .err()
        .unwrap();
    assert_eq!(error.code(), "schema.input.contract_mismatch");
}
