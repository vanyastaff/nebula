use nebula_schema::Schema;
use serde::Deserialize;

#[derive(Schema, Deserialize)]
struct Invalid {
    #[serde(skip)]
    #[property(input(secret))]
    text: String,
}

fn main() {}
