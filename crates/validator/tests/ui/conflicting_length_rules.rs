//! `exact_length` conflicts with `min_length` / `max_length`.

use nebula_validator::Validator;

#[derive(Validator)]
struct Bad {
    #[validate(exact_length = 5, min_length = 3)]
    name: String,
}

fn main() {}
