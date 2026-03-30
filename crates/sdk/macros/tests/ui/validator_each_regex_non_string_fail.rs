use nebula_macros::Validator;

#[derive(Validator)]
struct InvalidEachRegexNonString {
    #[validate(each(regex = r"^\\d+$"))]
    ids: Vec<u64>,
}

fn main() {}