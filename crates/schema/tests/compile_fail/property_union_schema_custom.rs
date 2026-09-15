use nebula_schema::Schema;

#[derive(Schema)]
#[schema(custom = "validate")]
enum Choice {
    First,
}

fn main() {}
