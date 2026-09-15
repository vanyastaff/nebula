use nebula_schema::Schema;

#[derive(Schema)]
enum Invalid {
    #[property(input(secret))]
    Token { value: String },
}

fn main() {}
