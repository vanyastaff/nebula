//! A fingerprinted field whose type names a hash-ordered set is refused: its
//! serialized order follows the per-process hash seed, so the configuration
//! fingerprint would not be stable. `HashMap` (an object, keys sorted) and a
//! skipped set are accepted.

use std::collections::{HashMap, HashSet};

use nebula_resource::ResourceConfig;

#[derive(Clone, ResourceConfig)]
#[config(schema = external)]
struct BareSet {
    hosts: HashSet<String>,
}

#[derive(Clone, ResourceConfig)]
#[config(schema = external)]
struct NestedSet {
    hosts: Option<Vec<std::collections::HashSet<u16>>>,
}

#[derive(Clone, ResourceConfig)]
#[config(schema = external)]
struct TupleSet(String, HashSet<u8>);

#[derive(Clone, ResourceConfig)]
#[config(schema = external)]
struct Accepted {
    headers: HashMap<String, String>,
    #[config(skip_fingerprint)]
    labels: HashSet<String>,
}

nebula_resource::impl_empty_has_schema!(BareSet, NestedSet, TupleSet, Accepted);

fn main() {}
