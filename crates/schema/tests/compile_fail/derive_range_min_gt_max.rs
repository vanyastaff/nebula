use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[validate(range(100..=50))]
    count: u32,
}

fn main() {
    let _ = Bad::schema();
}
