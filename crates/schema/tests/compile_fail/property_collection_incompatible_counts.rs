use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(items(min = 1)))]
    values: String,
}

fn main() {}
