use nebula_schema::Schema;

// Reserving the same key twice is a typo — the duplicate is a dead entry, so the
// derive rejects it rather than silently accepting a no-op reservation.
#[derive(Schema)]
#[schema(reserved("old_key", "old_key"))]
#[allow(dead_code)]
struct Bad {
    name: String,
}

fn main() {
    let _ = Bad::schema();
}
