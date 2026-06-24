use nebula_schema::Schema;

// An `emit_as` key must be a valid `FieldKey` (ASCII identifier) — a `kebab-case`
// key is a spanned compile error, not a runtime panic.
#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[field(emit_as = "has-dash")]
    field: String,
}

fn main() {
    let _ = Bad::schema();
}
