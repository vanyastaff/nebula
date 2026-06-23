use nebula_schema::Schema;

#[derive(Schema)]
#[schema(reserved("name"))]
#[allow(dead_code)]
struct Bad {
    // `name` is reserved, so a field resolving to that key must be rejected —
    // reserving a key forbids reusing it for a different field.
    name: String,
}

fn main() {
    let _ = Bad::schema();
}
