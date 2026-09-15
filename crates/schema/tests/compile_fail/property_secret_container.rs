use nebula_schema::Schema;

#[derive(Schema)]
#[property(input(secret))]
struct Invalid {
    text: String,
}

fn main() {}
