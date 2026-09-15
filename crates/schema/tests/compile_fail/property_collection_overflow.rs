use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(items(max = 4294967296)))]
    values: Vec<u8>,
}

fn main() {}
