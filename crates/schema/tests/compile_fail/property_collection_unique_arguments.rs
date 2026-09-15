use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(unique(false)))]
    values: Vec<u8>,
}

fn main() {}
