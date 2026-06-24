use nebula_schema::Schema;

// An `emit_as` key must not equal a canonical field key — two fields would emit
// under the same output key.
#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[field(emit_as = "target")]
    source: String,
    target: String,
}

fn main() {
    let _ = Bad::schema();
}
