//! `#[derive(Schema)]` rejects a variant whose resolved wire key is not a valid
//! `FieldKey`: `kebab-case` produces `foo-bar`, and a hyphen is illegal in a key
//! (variant keys are used as schema path segments).

use nebula_schema::Schema;

#[derive(serde::Serialize, Schema)]
struct Cfg {
    x: String,
}

#[derive(serde::Serialize, Schema)]
#[serde(rename_all = "kebab-case")]
enum Bad {
    FooBar(Cfg),
}

fn main() {}
