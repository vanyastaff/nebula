use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[field(no_expression)]
    #[property(input(expressions = allowed))]
    text: String,
}

fn main() {}
