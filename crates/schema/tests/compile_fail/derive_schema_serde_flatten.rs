use nebula_schema::Schema;

#[derive(Schema, serde::Serialize)]
struct Inner {
    a: String,
}

// `#[serde(flatten)]` is not yet spliced into the schema — the derive must reject
// it with a clear compile error rather than silently dropping the nested fields.
#[derive(Schema, serde::Serialize)]
struct Outer {
    #[serde(flatten)]
    inner: Inner,
}

fn main() {}
