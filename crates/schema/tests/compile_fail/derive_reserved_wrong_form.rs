use nebula_schema::Schema;

// `reserved` is a list-form option; the assignment form is a usage error, and the
// derive must say so (point at the list syntax, not claim the option is unknown).
#[derive(Schema)]
#[schema(reserved = "old_key")]
#[allow(dead_code)]
struct Bad {
    name: String,
}

fn main() {
    let _ = Bad::schema();
}
