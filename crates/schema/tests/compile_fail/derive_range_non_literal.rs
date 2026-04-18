use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    // A non-literal range bound — derive must surface a clear error
    // instead of silently dropping the bound.
    #[validate(range("low".."high"))]
    count: u32,
}

fn main() {
    let _ = Bad::schema();
}
