use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[field(no_expression, expression_required)]
    name: String,
}

fn main() {
    let _ = Bad::schema();
}
