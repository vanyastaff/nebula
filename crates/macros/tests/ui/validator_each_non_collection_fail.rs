use nebula_macros::Validator;

#[derive(Validator)]
struct InvalidEachNonCollection {
    #[validate(each(email))]
    email: String,
}

fn main() {}