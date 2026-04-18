use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    // A bool field with a string default — derive must reject this,
    // otherwise we'd ship `Value::String("42")` as a bool default and
    // hit validation errors on first use.
    #[param(default = "42")]
    flag: bool,
}

fn main() {
    let _ = Bad::schema();
}
