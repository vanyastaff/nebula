use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(items(min = 8, max = 1)))]
    values: Vec<u8>,
}

fn main() {}
