use nebula_macros::Validator;

#[derive(Validator)]
struct InvalidEachNonString {
    #[validate(each(email))]
    scores: Vec<u32>,
}

fn main() {}