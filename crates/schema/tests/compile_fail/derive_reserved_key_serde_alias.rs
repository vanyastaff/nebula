use nebula_schema::Schema;

// A `#[serde(alias = "old")]` is an alternative wire key serde still deserializes
// into the field, so reserving that key while a field aliases it would reintroduce
// the misread the reservation guards against — the derive must reject it.
#[derive(Schema, serde::Deserialize)]
#[schema(reserved("old"))]
#[allow(dead_code)]
struct Bad {
    #[serde(alias = "old")]
    current: String,
}

fn main() {
    let _ = Bad::schema();
}
