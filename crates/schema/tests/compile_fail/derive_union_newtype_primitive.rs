//! `#[derive(Schema)]` rejects a newtype variant over a non-struct payload — a
//! primitive (or `Vec` / `Option`) payload has no field schema yet; wrap it in a
//! `#[derive(Schema)]` struct.

use nebula_schema::Schema;

#[derive(Schema)]
enum Bad {
    A(String),
}

fn main() {}
