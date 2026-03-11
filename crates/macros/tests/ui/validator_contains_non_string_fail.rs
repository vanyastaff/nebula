use nebula_macros::Validator;

#[derive(Validator)]
struct InvalidContainsType {
    #[validate(contains = "foo")]
    count: u32,
}

fn main() {}