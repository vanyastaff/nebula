use nebula_schema::Schema;

// Field `a` emits under `wire` (write-alias) while field `b` reads `wire`
// (serde alias): a wire round-trip would move data between the two fields.
#[derive(Schema, serde::Deserialize)]
#[allow(dead_code)]
struct Bad {
    #[field(write_alias = "wire")]
    a: String,
    #[serde(alias = "wire")]
    b: String,
}

fn main() {
    let _ = Bad::schema();
}
