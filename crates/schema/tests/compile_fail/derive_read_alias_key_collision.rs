use nebula_schema::Schema;

// A `#[serde(alias)]` read-alias must not equal another field's canonical key —
// it would steal that field's identity on input.
#[derive(Schema, serde::Deserialize)]
#[allow(dead_code)]
struct Bad {
    #[serde(alias = "email")]
    name: String,
    email: String,
}

fn main() {
    let _ = Bad::schema();
}
