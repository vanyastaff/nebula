use nebula_schema::{FieldCollector, Schema};

fn main() {
    // `item` is on ListBuilder, not GroupBuilder.
    let _ = Schema::builder()
        .group("g", |g| g.item("x"))
        .build();
}
