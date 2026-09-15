use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(input(secret))]
    text: String,
}

fn main() {}
