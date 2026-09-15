use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[validate(length(min = 1))]
    #[property(validate(items(min = 1)))]
    values: Vec<u8>,
}

fn main() {}
