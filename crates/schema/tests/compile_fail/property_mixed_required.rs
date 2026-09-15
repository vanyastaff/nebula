use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[validate(required)]
    #[property(input(required))]
    text: String,
}

fn main() {}
