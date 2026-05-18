use nebula_storage_port::StorageError;

#[test]
fn not_found_is_constructible_and_display() {
    let e = StorageError::not_found("execution", "01J");
    assert!(format!("{e}").contains("execution"));
}

#[test]
fn scope_violation_distinct_from_not_found() {
    let a = StorageError::not_found("execution", "x");
    let b = StorageError::ScopeViolation {
        entity: "execution",
    };
    assert_ne!(std::mem::discriminant(&a), std::mem::discriminant(&b));
}
