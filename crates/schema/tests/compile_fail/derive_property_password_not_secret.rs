use nebula_schema::Schema;

#[derive(Schema)]
struct Bad {
    #[property(display(widget = password))]
    token: String,
}

fn main() {}
