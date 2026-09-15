use nebula_schema::{PropertyCollector, Schema};

fn main() {
    // `min` belongs to NumberField, not StringField/StringBuilder.
    let _ = Schema::builder().string(nebula_schema::field_key!("name"), |s| s.min(1)).build();
}
