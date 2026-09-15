use nebula_schema::Schema;

#[derive(Schema)]
struct Bad {
    #[property(layout(label = "Name"))]
    name: String,
}

fn main() {}
