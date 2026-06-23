use nebula_schema::Schema;

// A `write_alias` must be a valid `FieldKey` (ASCII identifier) — a `kebab-case`
// alias is a spanned compile error, not a runtime panic.
#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[field(write_alias = "has-dash")]
    field: String,
}

fn main() {
    let _ = Bad::schema();
}
