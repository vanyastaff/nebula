use nebula_storage_port::StorageError;

/// Value the decoder must not re-publish. Structured so a substring match
/// cannot false-positive on unrelated framework text.
const MARKER: &str = "MARKER-9f3a-secret";

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

/// **A decode failure carried by [`StorageError`] must not re-publish the value
/// that failed to decode.**
///
/// `StorageError`'s `Display` reaches `%error` log fields, dispatch reason
/// strings, and — through `Internal` — the durable `execution_control_queue`
/// `error_message`, and every `?` on a decode inside a storage adapter lands in
/// [`StorageError::Serialization`]. Forwarding `serde_json`'s own `Display`
/// there would carry the stored payload into all three; on a pre-envelope
/// execution row that payload is the free-text provider error the typed failure
/// envelope exists to remove.
///
/// The premise assertion is load-bearing: it proves the decoder really does
/// quote the value, so the redaction assertion below cannot pass vacuously.
#[test]
fn serialization_error_never_publishes_the_value_that_failed_to_decode() {
    let decode_error = serde_json::from_value::<bool>(serde_json::json!(MARKER))
        .expect_err("a JSON string is not a bool");

    assert!(
        decode_error.to_string().contains(MARKER),
        "premise: serde_json's Display quotes the offending value; it rendered: {decode_error}"
    );

    let rendered = StorageError::from(decode_error).to_string();
    assert!(
        !rendered.contains(MARKER),
        "StorageError::Serialization must not re-publish the decoded value; it rendered: {rendered}"
    );
    assert!(
        rendered.contains("data error"),
        "the summary must still name the failure kind from the parser's Category; \
         it rendered: {rendered}"
    );
}
