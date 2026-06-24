use nebula_schema::Schema;

// An `emit_as` re-emits a reserved key on output — reusing a reserved wire
// key for a different field's projected value is rejected.
#[derive(Schema)]
#[schema(reserved("old_out"))]
#[allow(dead_code)]
struct Bad {
    #[field(emit_as = "old_out")]
    current: String,
}

fn main() {
    let _ = Bad::schema();
}
