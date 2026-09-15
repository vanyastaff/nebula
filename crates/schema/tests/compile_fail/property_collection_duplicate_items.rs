use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(items(min = 1), items(max = 8)))]
    values: Vec<u8>,
}

fn main() {}
