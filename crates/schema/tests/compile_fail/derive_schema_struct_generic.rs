//! `#[derive(Schema)]` rejects generic structs too (the struct path shares the
//! same `OnceLock`-per-monomorphization hazard as the enum path).

use nebula_schema::Schema;

#[derive(Schema)]
struct Bad<T> {
    field: T,
}

fn main() {}
