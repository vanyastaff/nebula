//! `#[derive(Schema)]` rejects internally-tagged enums (`#[serde(tag = ..)]`
//! without `content`): the tag inlines into the payload's field namespace, so the
//! schema cannot keep the variant key disjoint from the payload keys.

use nebula_schema::Schema;

#[derive(serde::Serialize, Schema)]
#[serde(tag = "kind")]
enum Bad {
    A { x: i64 },
}

fn main() {}
