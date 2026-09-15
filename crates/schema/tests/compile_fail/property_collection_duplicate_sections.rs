use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(items(min = 1)))]
    #[property(validate(unique))]
    values: Vec<u8>,
}

fn main() {}
