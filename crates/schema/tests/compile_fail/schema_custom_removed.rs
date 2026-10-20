use nebula_schema::Schema;

#[derive(Schema)]
#[schema(custom = "validate")]
struct Guarded {
    label: String,
}

fn main() {}
