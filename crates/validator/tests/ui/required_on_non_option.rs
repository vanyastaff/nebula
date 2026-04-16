//! `required` is only valid on `Option<T>` fields.

use nebula_validator::Validator;

#[derive(Validator)]
struct Bad {
    #[validate(required)]
    name: String,
}

fn main() {}
