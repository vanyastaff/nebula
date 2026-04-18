use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    // i128 doesn't round-trip through serde_json::Number — the derive
    // must reject this at macro-expansion time.
    count: i128,
}

fn main() {
    let _ = Bad::schema();
}
