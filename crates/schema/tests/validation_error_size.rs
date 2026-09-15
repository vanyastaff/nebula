use std::mem::size_of;

use nebula_schema::ValidationError;

#[test]
fn validation_error_stays_boxed_and_small() {
    assert!(
        size_of::<ValidationError>() <= 32,
        "ValidationError must stay pointer-sized enough for Result<T, ValidationError>; \
         box payload-bearing details instead of adding inline fields"
    );
}
