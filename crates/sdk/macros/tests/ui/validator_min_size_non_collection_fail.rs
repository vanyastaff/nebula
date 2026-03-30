use nebula_macros::Validator;

#[derive(Validator)]
pub struct InvalidCollectionSize {
    #[validate(min_size = 1)]
    name: String,
}

fn main() {}
