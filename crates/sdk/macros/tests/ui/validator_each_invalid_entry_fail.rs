use nebula_macros::Validator;

#[derive(Validator)]
struct InvalidEachEntry {
    #[validate(each(42))]
    names: Vec<String>,
}

fn main() {}