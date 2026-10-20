use nebula_schema::Schema;

#[derive(Schema)]
#[schema(reserved("old"))]
enum Choice {
    First,
}

fn main() {}
