use nebula_macros::Validator;

#[derive(Validator)]
pub struct InvalidInput {
    #[validate(min_length = "short")]
    username: String,
}

fn main() {}
