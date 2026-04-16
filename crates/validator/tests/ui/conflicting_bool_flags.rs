//! `is_true` and `is_false` cannot coexist.

use nebula_validator::Validator;

#[derive(Validator)]
struct Bad {
    #[validate(is_true, is_false)]
    flag: bool,
}

fn main() {}
