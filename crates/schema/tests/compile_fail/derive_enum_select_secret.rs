use nebula_schema::{EnumSelect, Schema};

#[derive(EnumSelect)]
enum Color {
    Red,
}

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[field(secret, enum_select)]
    c: Color,
}

fn main() {
    let _ = Bad::schema();
}
