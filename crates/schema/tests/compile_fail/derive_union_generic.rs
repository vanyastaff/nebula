//! `#[derive(Schema)]` rejects generic enums — the generated `schema()` caches one
//! `OnceLock` shared across every monomorphization, so all but the first `T` would
//! get the wrong schema.

use nebula_schema::Schema;

#[derive(Schema)]
enum Bad<T> {
    A(T),
}

fn main() {}
