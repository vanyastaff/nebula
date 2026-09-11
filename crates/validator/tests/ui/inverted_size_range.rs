//! Literal inverted collection size ranges must fail during macro expansion.

use nebula_validator::Validator;

#[derive(Validator)]
struct Bad {
    #[validate(size_range(min = 5, max = 2))]
    values: Vec<u8>,
}

fn main() {}
