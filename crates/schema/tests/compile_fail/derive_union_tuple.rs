//! `#[derive(Schema)]` rejects tuple variants with more than one field — a
//! positional array has no named-field schema.

use nebula_schema::Schema;

#[derive(Schema)]
enum Bad {
    A(i64, i64),
}

fn main() {}
