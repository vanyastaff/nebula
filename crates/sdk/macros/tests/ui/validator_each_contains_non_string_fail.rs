use nebula_macros::Validator;

#[derive(Validator)]
struct InvalidEachContainsNonString {
    #[validate(each(contains = "1"))]
    values: Vec<u32>,
}

fn main() {}
