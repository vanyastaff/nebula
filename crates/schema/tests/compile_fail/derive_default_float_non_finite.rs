use nebula_schema::Schema;

#[derive(Schema)]
#[allow(dead_code)]
struct Bad {
    // A float literal that overflows to `f64::INFINITY`. The derive must reject
    // it at expansion time — otherwise the generated `Number::from_f64(..)`
    // would be `None` and panic in the consuming crate at runtime.
    #[field(default = 1e400)]
    ratio: f64,
}

fn main() {
    let _ = Bad::schema();
}
