use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[validate(length(min = 100, max = 50))]
    name: String,
}

fn main() {
    let _ = Bad::schema();
}
