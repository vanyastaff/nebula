//! Invalid regex patterns must be rejected at macro-time.

use nebula_validator::Validator;

#[derive(Validator)]
struct Bad {
    #[validate(regex = "[invalid")]
    code: String,
}

fn main() {}
