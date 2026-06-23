use nebula_schema::Schema;

#[derive(Schema)]
// A reserved key that is not a valid `FieldKey` can never match a real key, so
// the reservation would be silently inert — the derive must reject the typo.
#[schema(reserved("not a valid key"))]
#[allow(dead_code)]
struct Bad {
    name: String,
}

fn main() {
    let _ = Bad::schema();
}
