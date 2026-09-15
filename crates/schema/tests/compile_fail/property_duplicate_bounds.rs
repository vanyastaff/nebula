use nebula_schema::Schema;

#[derive(Schema)]
struct Invalid {
    #[property(validate(length(min = 8), length(max = 64)))]
    text: String,
}

fn main() {}
