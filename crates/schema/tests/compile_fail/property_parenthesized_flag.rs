use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(input(required(false)))]
    text: String,
}

fn main() {}
