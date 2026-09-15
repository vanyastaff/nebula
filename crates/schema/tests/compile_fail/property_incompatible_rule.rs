use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(range(1..=3)))]
    text: String,
}

fn main() {}
