use nebula_schema::Schema;

// The reservation is checked against the *resolved* wire key, so a container
// `#[serde(rename_all = ..)]` that maps a field onto a reserved key still
// collides (the resolved key `myField`, not the raw ident `my_field`).
#[derive(Schema, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[schema(reserved("myField"))]
#[allow(dead_code)]
struct Bad {
    my_field: String,
}

fn main() {
    let _ = Bad::schema();
}
