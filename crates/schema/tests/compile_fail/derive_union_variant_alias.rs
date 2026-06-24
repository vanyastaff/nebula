//! `#[derive(Schema)]` rejects `#[serde(alias = ..)]` on an enum variant — a union
//! records one wire discriminant per variant, so an alias serde would still
//! deserialize becomes a key schema validation rejects (a C1 desync).

use nebula_schema::Schema;

#[derive(serde::Serialize, Schema)]
struct Cfg {
    x: String,
}

#[derive(serde::Serialize, Schema)]
enum Bad {
    #[serde(alias = "Alt")]
    Primary(Cfg),
}

fn main() {}
