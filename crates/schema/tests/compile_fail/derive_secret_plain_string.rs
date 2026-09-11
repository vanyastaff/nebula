use nebula_schema::{HasSchema, Schema};
use serde::Deserialize;

#[derive(Deserialize, Schema)]
struct PlainSecret {
    #[field(secret)]
    token: String,
}

fn main() {
    let _ = PlainSecret::schema();
}
