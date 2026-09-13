//! The collection-oriented `length(...)` spelling enforces the same bound ordering.

use nebula_validator::Validator;

#[derive(Validator)]
struct Bad {
    #[validate(length(min = 5, max = 2))]
    values: Vec<u8>,
}

fn main() {}
