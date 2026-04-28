use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[param(secret, multiline)]
    api_key: String,
}

fn main() {
    let _ = Bad::schema();
}
