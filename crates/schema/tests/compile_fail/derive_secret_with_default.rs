use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[param(secret, default = "hardcoded-secret")]
    api_key: String,
}

fn main() {
    let _ = Bad::schema();
}
