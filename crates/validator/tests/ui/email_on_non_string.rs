//! `email` requires a `String` or `Option<String>` field.

use nebula_validator::Validator;

#[derive(Validator)]
struct Bad {
    #[validate(email)]
    age: i32,
}

fn main() {}
