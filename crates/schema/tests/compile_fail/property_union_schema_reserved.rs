use nebula_schema::Schema;

#[derive(Schema)]
#[schema(reserved("Old"))]
enum Choice {
    Old,
    New,
}

fn main() {}
