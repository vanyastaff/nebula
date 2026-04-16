//! `Validator` derive is not supported on enums.

use nebula_validator::Validator;

#[derive(Validator)]
enum Bad {
    A,
    B,
}

fn main() {}
