use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(unique))]
    values: Option<u8>,
}

fn main() {}
