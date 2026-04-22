use nebula_schema::Schema;

#[derive(Schema)]
#[schema(unknown_option = "nope")]
struct Bad {
    x: String,
}

fn main() {}
