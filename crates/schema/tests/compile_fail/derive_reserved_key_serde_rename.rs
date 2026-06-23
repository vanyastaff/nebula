use nebula_schema::Schema;

// The reservation is checked against the *resolved* wire key, not the raw field
// ident: a field serde-renamed onto a reserved key still collides.
#[derive(Schema, serde::Serialize)]
#[schema(reserved("legacy"))]
#[allow(dead_code)]
struct Bad {
    #[serde(rename = "legacy")]
    current: String,
}

fn main() {
    let _ = Bad::schema();
}
