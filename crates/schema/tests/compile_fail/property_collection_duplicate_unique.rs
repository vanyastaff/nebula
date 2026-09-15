use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(unique, unique))]
    values: Vec<u8>,
}

fn main() {}
