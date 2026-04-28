use nebula_schema::{FieldCollector, Schema};
use serde_json::json;

fn main() {
    // `option` belongs to SelectField, not BooleanField/BooleanBuilder.
    let _ = Schema::builder()
        .boolean(nebula_schema::field_key!("flag"), |b| b.option(json!(1), "X"))
        .build();
}
