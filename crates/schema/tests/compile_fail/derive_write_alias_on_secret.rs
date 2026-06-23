use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[field(secret, write_alias = "exposed")]
    api_key: String,
}

fn main() {
    let _ = Bad::schema();
}
