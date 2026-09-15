use nebula_schema::Schema;

#[derive(Schema)]
struct Payload {
    text: String,
}

#[derive(Schema)]
enum Invalid {
    Token(#[property(input(secret))] Payload),
}

fn main() {}
