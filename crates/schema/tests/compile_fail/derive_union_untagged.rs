//! `#[derive(Schema)]` rejects `#[serde(untagged)]` enums — an untagged union has
//! no discriminant key, so the schema cannot reproduce its wire shape (C1).

use nebula_schema::Schema;

#[derive(serde::Serialize, Schema)]
struct Cfg {
    x: String,
}

#[derive(serde::Serialize, Schema)]
#[serde(untagged)]
enum Bad {
    A(Cfg),
}

fn main() {}
