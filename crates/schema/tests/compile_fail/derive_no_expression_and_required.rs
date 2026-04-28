use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    #[param(no_expression, expression_required)]
    name: String,
}

fn main() {
    let _ = Bad::schema();
}
