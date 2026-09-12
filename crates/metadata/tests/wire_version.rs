//! Historical unversioned metadata is not evidence for the new shared contract.

use nebula_metadata::RecordedBaseMetadata;
use nebula_schema::ValidSchema;
use serde_json::json;

#[test]
fn rejects_unversioned_shared_record() {
    let historical = json!({
        "key": "example.action",
        "name": "Example",
        "description": "",
        "schema": ValidSchema::empty(),
    });
    serde_json::from_value::<RecordedBaseMetadata<String>>(historical)
        .expect_err("unversioned historical evidence must not inherit new metadata semantics");
}
