use nebula_schema::Schema;

#[derive(Schema)]
struct ExampleHint {
    #[property(display(example = "hello"))]
    value: String,
}

fn main() {}
