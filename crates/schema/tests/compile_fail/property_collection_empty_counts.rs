use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(items()))]
    values: Vec<u8>,
}

fn main() {}
