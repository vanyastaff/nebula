use nebula_schema::{FieldCollector, Schema};
use serde_json::json;

fn main() {
    // `option` belongs to SelectField, not BooleanField/BooleanBuilder.
    let _ = Schema::builder()
        .boolean("flag", |b| b.option(json!(1), "X"))
        .build();
}
