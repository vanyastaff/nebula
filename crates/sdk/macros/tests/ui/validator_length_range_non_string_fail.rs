use nebula_macros::Validator;

#[derive(Validator)]
struct InvalidLengthRange {
    #[validate(length_range(min = 1, max = 3))]
    count: u32,
}

fn main() {}
