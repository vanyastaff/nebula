use nebula_schema::{EnumSelect, Schema};

#[derive(EnumSelect)]
enum Color {
    Red,
}

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[param(enum_select)]
    #[validate(url)]
    c: Color,
}

fn main() {
    let _ = Bad::schema();
}
