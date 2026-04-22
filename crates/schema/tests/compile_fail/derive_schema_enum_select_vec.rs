use nebula_schema::{EnumSelect, Schema};

#[derive(EnumSelect)]
#[allow(dead_code)]
enum Method {
    Get,
}

#[derive(Schema)]
struct Bad {
    #[param(enum_select)]
    items: Vec<Method>,
}

fn main() {}
