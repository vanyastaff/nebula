use nebula_schema::Schema;

// A `write_alias` must not equal a canonical field key — two fields would emit
// under the same output key.
#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[field(write_alias = "target")]
    source: String,
    target: String,
}

fn main() {
    let _ = Bad::schema();
}
