use nebula_macros::Validator;

#[derive(Validator)]
struct InvalidBooleanType {
    #[validate(is_true)]
    status: String,
}

fn main() {}